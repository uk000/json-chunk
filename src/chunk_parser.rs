use crate::parser::{JSONEvent, JSONEventGenerator, JSONEventWrapper};
use serde_json::Value;
use std::fmt;
use std::{collections::HashMap, mem};

#[derive(Debug, Clone)]
pub struct JSONPathTracker {
    pub json_path: String,
    pub path_vector: Vec<String>,
    pub output_key: Option<String>,
    pub max_value_length: usize,
    pub matched_depth: usize,
    pub array_nesting: usize,
    pub skipped_depth: usize,
    pub current_key: Option<String>,
    /// Set to true once this tracker's first match is recorded (used with stop_at_first_match).
    pub done: bool,
    /// Non-zero while buffering a matched child object/array JSON blob.
    pub collecting_depth: usize,
    /// Accumulates raw JSON bytes for the current child-object/array collection.
    /// Bytes for every event (including structural chars) are appended here while
    /// collecting_depth > 0.  Cleared when a new collection starts.
    pub collect_buffer: Vec<u8>,
    /// Set to true once this tracker's collected value exceeds the given max_size
    pub overflow: bool,
}

pub struct JSONChunkParser {
    pub scratch_buffer: Vec<u8>,
    pub json_parser: JSONEventGenerator,
    /// When true, halt after the first match found across all paths.
    pub stop_at_first_match: bool,
    /// One tracker per target path; all advance independently on the same stream.
    pub tracked_fields: HashMap<String, JSONPathTracker>,
    /// Results collected, keyed by either matched path or overridden path.
    pub matches_found: HashMap<String, Value>,
}

impl JSONChunkParser {
    pub fn add_search_field(&mut self, json_path: String, output: Option<String>, max_size: usize) {
        let key = json_path.clone();
        let path = json_path.split('.').map(String::from).collect();
        let tracker = JSONPathTracker {
            json_path,
            path_vector: path,
            output_key: output,
            max_value_length: max_size,
            matched_depth: 0,
            array_nesting: 0,
            skipped_depth: 0,
            current_key: None,
            done: false,
            collecting_depth: 0,
            collect_buffer: Vec::new(),
            overflow: false,
        };
        self.tracked_fields.insert(key, tracker);
    }

    pub fn process_chunks(&mut self, chunks: &Vec<Vec<u8>>) {
        let total = chunks.len();
        for (i, chunk) in chunks.iter().enumerate() {
            self.process_chunk(chunk, i == total - 1);
            if self.is_all_found() {
                break;
            }
        }
    }

    pub fn process_chunk(&mut self, chunk: &Vec<u8>, end_of_stream: bool) {
        self.scratch_buffer.extend_from_slice(&chunk);

        let mut cursor = 0;
        let mut short_circuit = false;

        loop {
            let slice_to_parse = &self.scratch_buffer[cursor..];
            let JSONEventWrapper {
                consumed_bytes,
                event,
            } = self.json_parser.parse_next(slice_to_parse, end_of_stream);
            let event_start = cursor;
            cursor += consumed_bytes;
            let b: Vec<u8> = self.scratch_buffer[event_start..cursor].to_vec();

            let ev = match event {
                None => {
                    // No event produced, but bytes may have been consumed (e.g. a ':' or ','
                    // token that precedes the next value and straddles a chunk boundary).
                    // Append those bytes to any active collect_buffers so they are not lost
                    // when scratch_buffer is drained.
                    if consumed_bytes > 0 {
                        self.feed_trackers(&b);
                    }
                    break;
                }
                Some(Err(_)) => break,
                Some(Ok(ev)) => ev,
            };

            // ── Extract owned data before mutably borrowing trackers ─────────────
            let obj_key: Option<String> = if let JSONEvent::ObjectKey(k) = &ev {
                Some(k.to_string())
            } else {
                None
            };
            let leaf_val: Option<String> = match &ev {
                JSONEvent::String(v) | JSONEvent::Number(v) => Some(v.to_string()),
                JSONEvent::Boolean(b) => Some(b.to_string()),
                _ => None,
            };
            let is_start_object = matches!(&ev, JSONEvent::StartObject);
            let is_end_object = matches!(&ev, JSONEvent::EndObject);
            let is_start_array = matches!(&ev, JSONEvent::StartArray);
            let is_end_array = matches!(&ev, JSONEvent::EndArray);
            let is_eof = matches!(&ev, JSONEvent::Eof);
            drop(ev); // release borrow of scratch_buffer

            if is_eof {
                break;
            }

            // ── Fan the event out to every path tracker independently ─────────────
            // let mut chunk_matches: Vec<(usize, String)> = Vec::new();

            for (_, tracker) in &mut self.tracked_fields {
                // Per-tracker early exit: skip trackers that already found their match.
                if tracker.done {
                    continue;
                }
                // scratch_buffer and fields_to_search are disjoint fields; both borrows
                // are valid simultaneously under Rust's split-borrow rules.
                //If a tracker is already collecting, let it collect current chunk too.
                if tracker.is_collecting() {
                    tracker.collect(&b, false);
                    // If we're buffering a child JSON blob, accumulate bytes for this event
                    // THEN adjust the nesting depth.  The closing } or ] must land in the
                    // buffer before we emit the result.
                    tracker.move_collect_pointers(
                        is_start_object,
                        is_end_object,
                        is_start_array,
                        is_end_array,
                    );

                    if !tracker.is_collecting() {
                        tracker.finish();
                        //     let json_str = String::from_utf8_lossy(&tracker.collect_buffer).into_owned();
                        //     chunk_matches.push((i, json_str));
                    }
                    continue;
                }
                // If a tracker is not collecting at present and not finished yet,
                // let it check if it's interested in start collecting
                if obj_key.is_some() {
                    //a new field name appears
                    if !tracker.is_skipping() {
                        tracker.set_current_key(&obj_key);
                    }
                } else if is_start_object {
                    //an object starts, ask tracker if ineterested
                    if tracker.is_object_of_interest() {
                        tracker.collect_start_marker(&b);
                    }
                } else if is_end_object {
                    //an object ends, ask tracker to unwind
                    tracker.unwind(false);
                } else if is_start_array {
                    //an array starts, ask tracker if ineterested
                    if tracker.is_array_of_interest() {
                        tracker.collect_start_marker(&b);
                    }
                } else if is_end_array {
                    //an object ends, ask tracker to unwind
                    tracker.unwind(true);
                } else if let Some(ref v) = leaf_val {
                    //leaf value arrived and tracker wasn't collecting yet.
                    //check with tracker again if interested in collecting now
                    if tracker.will_collect() {
                        tracker.collect(v.as_bytes(), true);
                        tracker.finish();
                        // if !tracker.overflow {
                        //     chunk_matches.push((i, v.clone()));
                        // }
                    }
                }
            }

            // Mark each matched tracker as done (per-tracker stop) and collect results.
            // self.store_chunk_matches(chunk_matches);

            // Short-circuit the outer parse loop only when ALL trackers are done.
            if self.is_all_done() {
                short_circuit = true;
                break;
            }
        }

        self.scratch_buffer.drain(0..cursor);

        if end_of_stream || short_circuit {
            self.end_tracking();
            //if need to do something after all matches found, this is the place
        }
    }

    fn end_tracking(&mut self) {
        for (_, tracker) in &mut self.tracked_fields {
            tracker.finish();
            if tracker.has_data() {
                let buf = &tracker.collect_buffer;
                let json_value = match buf.first() {
                    // Object or array — parse as JSON directly.
                    Some(b'{') | Some(b'[') => serde_json::from_slice::<Value>(buf).ok(),
                    // Anything else: try JSON first (handles numbers, booleans, null),
                    // then fall back to treating the raw bytes as a plain string.
                    _ => serde_json::from_slice::<Value>(buf).ok().or_else(|| {
                        std::str::from_utf8(buf)
                            .ok()
                            .map(|s| Value::String(s.to_owned()))
                    }),
                };
                if let Some(v) = json_value {
                    self.matches_found.insert(tracker.json_path.clone(), v);
                }
            }
        }
    }

    fn feed_trackers(&mut self, b: &[u8]) {
        for (_, tracker) in &mut self.tracked_fields {
            tracker.collect(b, false);
        }
    }

    pub fn is_all_done(&self) -> bool {
        self.stop_at_first_match && self.tracked_fields.iter().all(|(_, t)| t.done)
    }

    pub fn is_all_found(&self) -> bool {
        self.tracked_fields.len() == self.matches_found.len()
    }

    pub fn get_field(&self, name: &str) -> JSONPathTracker {
        return self.tracked_fields[name].clone();
    }

    pub fn to_json(&mut self) -> Value {
        for (_, tracker) in &self.tracked_fields {
            if let Some(output_key) = &tracker.output_key {
                if let Some(value) = self.matches_found.get(&tracker.json_path) {
                    self.matches_found.insert(output_key.clone(), value.to_owned());
                    self.matches_found.remove(&tracker.json_path);
                }
            }
        }
        let taken_map = mem::take(&mut self.matches_found);
        let json = serde_json::to_value(taken_map).unwrap();
        json
    }
}

impl Default for JSONChunkParser {
    fn default() -> Self {
        Self {
            scratch_buffer: Vec::new(),
            json_parser: JSONEventGenerator::new(),
            tracked_fields: HashMap::new(),
            stop_at_first_match: true,
            matches_found: HashMap::new(),
        }
    }
}

impl Clone for JSONChunkParser {
    fn clone(&self) -> Self {
        // LowLevelJsonParser is stateful and not Clone; reset it on clone.
        Self {
            scratch_buffer: self.scratch_buffer.clone(),
            json_parser: JSONEventGenerator::new(),
            tracked_fields: self.tracked_fields.clone(),
            stop_at_first_match: self.stop_at_first_match,
            matches_found: self.matches_found.clone(),
        }
    }
}

impl fmt::Debug for JSONChunkParser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonStreamParser")
            .field("scratch_buffer_len", &self.scratch_buffer.len())
            .field("fields_to_search", &self.tracked_fields)
            .field("stop_at_first_match", &self.stop_at_first_match)
            .field("matches_found", &self.matches_found)
            .finish()
    }
}

impl JSONPathTracker {
    fn is_collecting(&self) -> bool {
        return self.collecting_depth > 0;
    }

    fn is_skipping(&self) -> bool {
        return self.skipped_depth > 0;
    }

    fn set_current_key(&mut self, key: &Option<String>) {
        self.current_key = key.clone();
    }

    fn collect(&mut self, b: &[u8], is_new: bool) {
        if self.overflow {
            return;
        }
        let mut will_collect = false;
        if is_new {
            self.collecting_depth = 1;
            self.collect_buffer.clear();
            will_collect = true;
        } else if self.collecting_depth > 0 && !self.done {
            will_collect = true;
        }
        if will_collect {
            self.collect_buffer.extend_from_slice(b);
            if (self.max_value_length > 0) && (self.collect_buffer.len() > self.max_value_length) {
                self.reset();
                self.overflow = true;
            }
        }
    }

    fn collect_start_marker(&mut self, b: &[u8]) {
        let l = b.len();
        self.collect(&b[l - 1..l], true);
    }

    fn is_array_of_interest(&mut self) -> bool {
        let k = self.current_key.take().unwrap_or_default();
        if self.skipped_depth > 0 {
            self.skipped_depth += 1;
        } else if self.matched_depth < self.path_vector.len()
            && !k.is_empty()
            && k == self.path_vector[self.matched_depth]
        {
            if self.matched_depth == self.path_vector.len() - 1 {
                // Path terminates at this array key – collect the whole array.
                // Use cursor-1..cursor to capture only the '[' itself, not any
                // preceding ':' that the parser consumed as part of this event.
                return true;
            } else {
                self.matched_depth += 1;
                self.array_nesting += 1;
            }
        } else {
            self.skipped_depth += 1;
        }
        false
    }

    fn is_object_of_interest(&mut self) -> bool {
        if self.skipped_depth > 0 {
            self.skipped_depth += 1;
        } else if self.array_nesting > 0 {
            self.array_nesting += 1;
        } else {
            let k = self.current_key.take().unwrap_or_default();
            if self.match_key(k) {
                return true;
            }
        }
        false
    }

    fn will_collect(&self) -> bool {
        if self.skipped_depth == 0
            && !self.path_vector.is_empty()
            && self.matched_depth == self.path_vector.len() - 1
        {
            if let Some(ref k) = self.current_key {
                if k == &self.path_vector[self.matched_depth] {
                    return true;
                }
            }
        }
        false
    }

    fn move_collect_pointers(
        &mut self,
        is_start_object: bool,
        is_end_object: bool,
        is_start_array: bool,
        is_end_array: bool,
    ) {
        if is_start_object || is_start_array {
            self.collecting_depth += 1;
        } else if is_end_object || is_end_array {
            self.collecting_depth -= 1;
        }
    }

    fn unwind(&mut self, array_only: bool) {
        if self.skipped_depth > 0 {
            self.skipped_depth -= 1;
        } else if self.array_nesting > 0 {
            self.array_nesting -= 1;
            if array_only && self.array_nesting == 0 && self.matched_depth > 0 {
                self.matched_depth -= 1;
            }
        } else if !array_only && self.matched_depth > 0 {
            self.matched_depth -= 1;
        }
    }

    fn match_key(&mut self, k: String) -> bool {
        if k.is_empty() {
            return false;
        }
        if self.matched_depth < self.path_vector.len() && k == self.path_vector[self.matched_depth]
        {
            if self.matched_depth == self.path_vector.len() - 1 {
                // Path terminates at this object key – collect the whole subtree.
                // Use cursor-1..cursor to capture only the '{' itself, not any
                // preceding ':' that the parser consumed as part of this event.
                self.collecting_depth = 1;
                self.collect_buffer.clear();
                return true;
            } else {
                self.matched_depth += 1;
                return false;
            }
        } else {
            // empty key = root object, pass through
            self.skipped_depth += 1;
            return false;
        }
    }

    fn finish(&mut self) {
        self.done = true;
    }

    fn has_data(&self) -> bool {
        return self.collect_buffer.len() > 0;
    }

    fn reset(&mut self) {
        self.matched_depth = 0;
        self.array_nesting = 0;
        self.skipped_depth = 0;
        self.current_key = None;
        self.done = false;
        self.collecting_depth = 0;
        self.collect_buffer.clear();
        self.overflow = false;
    }
}
