use std::collections::{HashMap};
use std::sync::{LazyLock, Mutex};
use serde_json::{json, Value};
use serde::Serialize;
use json_chunk::parser::{JSONEvent, JSONEventGenerator, JSONEventWrapper};
use json_chunk::chunk_parser::ChunkParser;

#[path = "print_helpers.rs"]
pub mod print_helpers;
use print_helpers::print_json_structure;

use crate::print_helpers::print_json_chunkinfo;

pub static SUCCESSORS: LazyLock<Mutex<HashMap<&'static str, &'static str>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

// -- helper functions

pub fn print_tracker_depths(i: usize, chunk: &Vec<u8>, parser: &ChunkParser) {
    println!("= Chunk {i}: {}",  String::from_utf8_lossy(chunk));
    print!("  -- Depth at chunk {i}: ");
    for (k, t) in &parser.tracked_fields  {
        print!("[{k}: {}] ", t.matched_depth);
    }
    println!("");
}

pub fn feed_chunks_to_parser(parser: &mut ChunkParser, chunks: &Vec<Vec<u8>>, verbose: bool) -> Option<usize> {
    let total: usize = chunks.len();
    let mut matched = 0;
    let mut overflown = 0;
    for (i, chunk) in chunks.iter().enumerate() {
        parser.process_chunk(chunk, i == total - 1);
        if verbose {
            println!("Processing chunk {}/{} bytes {}", i+1, chunks.len(), chunk.len());
            print_tracker_depths(i, chunk, parser);
            if parser.done_fields.len() > matched {
                println!("Matched so far: {:?}", parser.done_fields);
                matched = parser.done_fields.len();
            }
            if parser.overflowed_fields.len() > overflown {
                println!("Overflow so far: {:?}", parser.overflowed_fields);
                overflown = parser.overflowed_fields.len();
            }
            println!("======================== Chunk {i} End ======================================");
        }
        if parser.is_all_found() {
            return Some(i+1)
        }
    }
    return None
}

pub fn build_expected_kv<T: Serialize>(k: &str, v: T, expected: &mut HashMap<String, Value>) {
    expected.insert(k.to_string(), to_json_value(v));
}

pub fn build_expected_with_pos(path_map: &HashMap<String, (Option<String>, usize)>, chunks: &Vec<Vec<u8>>) -> HashMap<String, (usize,Value)> {
    let mut expected : HashMap<String, (usize,Value)> = HashMap::new();
    // Derive expected values by navigating each path in the full JSON.
    for (json_path, output) in path_map {
        let mut rem : Vec<u8> = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            let fields_vec: Vec<&str> = json_path.split('.').collect();
            let mut data = rem.to_vec();
            data.extend_from_slice(chunk.as_slice());
            let json_value = extract_json_value(&data, &fields_vec);
            if !json_value.is_null() {
                expected.insert(output.clone().0.unwrap_or(json_path.to_string()), (i+1, json_value));
                break
            }
            rem = data.to_vec();
        }
    }
    expected
}

pub fn build_expected(path_map: &HashMap<String, (Option<String>, usize)>, all_expected : &HashMap<String, Value>) -> HashMap<String, Value> {
    let mut expected : HashMap<String, Value> = HashMap::new();
    // Derive expected values by navigating each path in the full JSON.
    for (json_path, output) in path_map {
        let o_json_value = all_expected.get(json_path);
        if let Some(json_value) = o_json_value {
            expected.insert(output.clone().0.unwrap_or(json_path.to_string()), json_value.clone());
        }
    }
    expected
}

pub fn get_expected_field_names(path_map: &HashMap<String, (Option<String>, usize)>) -> (Vec<&str>, HashMap<&str, &str>) {
    let fields: Vec<Vec<&str>> = path_map.keys().map(|k| k.split('.').collect()).collect();
    let all_names: Vec<&str> = fields.iter().flat_map(|p| p.iter().copied()).collect();
    let mut successors: HashMap<&str, &str> = HashMap::new();
    let successors_lock = SUCCESSORS.lock().unwrap();
    for name in &all_names {
        if let Some(&value) = successors_lock.get(name) {
            successors.insert(name, value);
        }
    }
    (all_names, successors)
}

pub fn text_value(ch: char, value_len: usize, quote: char) -> String {
    let q = quote.to_string();
    format!("{q}{}{q}", rep(ch, value_len))
}

pub fn build_text(field_count: usize, value_len: usize, quote: char, kvsep: &str, sep: char, all_expected : &mut Option<&mut HashMap<String, Value>>) -> String {
    let mut flat_fields = String::new();
    let q = quote.to_string();
    for i in 0..field_count {
        let ch = (b'a' + (i % 26) as u8) as char;
        let key = format!("field_{i}");
        let value = text_value(ch, value_len, ' ');
        flat_fields.push_str(&format!("{q}{key}{q}{kvsep}{q}{value}{q}"));
        flat_fields.push(sep);
        if let Some(m) = all_expected {
            m.insert(key, to_json_value(value));
        }
    }
    flat_fields
}

pub fn build_flat_fields(field_count: usize, value_len: usize, all_expected : &mut HashMap<String, Value>) -> String {
    build_text(field_count, value_len, '"', ": ", ',', &mut Some(all_expected))
}

pub fn str_array(ch: char, item_count: usize, value_len: usize) -> String {
    let items: Vec<String> = (0..item_count).map(|_| text_value(ch, value_len, '"')).collect();
    format!("[ {} ]", items.join(","))
}

pub fn obj_array(kch: char, vch: char, item_count: usize, value_len: usize) -> String {
    let items: Vec<String> = (0..item_count)
        .map(|_| format!("{{\"name\":{},\"value\":{}}}", text_value(kch, value_len, '"'), text_value(vch, value_len, '"')))
        .collect();
    format!("[ {} ]", items.join(","))
}

pub fn set_larger_json_successors() {
    *SUCCESSORS.lock().unwrap() = [
        ("field_0", "field_1"),
        ("field_1", "field_2"),
        ("field_2", "field_3"),
        ("field_3", "field_4"),
        ("field_4", "field_5"),
        ("field_5", "field_6"),
        ("field_6", "field_7"),
        ("field_7", "field_8"),
        ("field_8", "field_9"),
        ("field_9", "field_10"),
        ("field_10", "field_11"),
        ("field_11", "field_12"),
        ("field_12", "field_13"),
        ("field_13", "field_14"),
        ("field_14", "field_15"),
        ("field_15", "field_16"),
        ("field_16", "field_17"),
        ("field_17", "field_18"),
        ("field_18", "field_19"),
        ("field_19", "field_20"),
        ("field_20", "field_21"),
        ("field_21", "field_22"),
        ("field_22", "field_23"),
        ("field_23", "field_24"),
        ("field_24", "field_25"),
        ("field_25", "field_26"),
        ("field_26", "field_27"),
        ("field_27", "field_28"),
        ("field_28", "field_29"),
        ("field_29", "field_30"),
        ("field_30", "field_31"),
        ("metadata", "author"),
        ("author", "version"),
        ("version", "stats"),
        ("stats", "views"),
        ("views", "details"),
        ("details", "regions"),
        ("regions", "locale"),
        ("locale", "name"),
        ("name", "tags"),
        ("tags", "items"),
        ("items", "config"),
        ("config", "key"),
        ("key", "values"),
        ("values", "signature"),
        ("signature", "timestamp"),
    ]
    .into_iter().collect();
}

pub fn build_large_json(field_count: usize, value_len: usize, all_expected : &mut HashMap<String, Value>, verbose: bool) -> Vec<u8> {
    // ── flat scalar fields: field_0 … field_{N-1} ───────────────────────────
    let flat_fields = build_flat_fields(field_count, value_len, all_expected);
    // flat_fields always ends with ',' — the nested section follows
    let small_json = &mut build_small_json(field_count, value_len, all_expected, verbose);
    if small_json.len() >= 2 {
        small_json.pop();          // Remove last
        small_json.drain(0..1);    // Efficiently clear the first element
    } else {
        small_json.clear();
    }
    let json = format!(
        concat!(
            "{{",
            "{flat}",                   // field_0 … field_{N-1} (each ending with ',')
            "{small_json}",
            "}}"
        ),
        flat     = flat_fields,
        small_json = String::from_utf8_lossy(small_json),
    );
    let json_bytes = json.into_bytes();
    set_larger_json_successors();
    json_bytes
}

pub fn build_small_json(field_count: usize, value_len: usize, all_expected : &mut HashMap<String, Value>, verbose: bool) -> Vec<u8> {
    *SUCCESSORS.lock().unwrap() = [
        ("metadata", "author"),
        ("author", "version"),
        ("version", "stats"),
        ("stats", "views"),
        ("views", "details"),
        ("details", "regions"),
        ("regions", "locale"),
        ("locale", "name"),
        ("name", "tags"),
        ("tags", "items"),
        ("items", "config"),
        ("config", "key"),
        ("key", "values"),
        ("values", "signature"),
        ("signature", "timestamp"),
    ]
    .into_iter().collect();

    let author   = text_value('A', value_len, ' ');
    let version  = text_value('B', value_len, ' ');
    let views    = obj_array('C', 'D', field_count, value_len);
    let regions   = str_array('E', field_count, value_len);
    let locale   = text_value('F', value_len, ' ');
    let name   = text_value('G', value_len, ' ');
    let tags     = str_array('H', field_count, value_len);
    let items    = obj_array('I', 'J', field_count, value_len);
    let key  = text_value('K', value_len, ' ');
    let value1      = text_value('L', value_len, ' ');
    let value2      = text_value('M', value_len, ' ');
    let signature = str_array('N', field_count, value_len);
    let timestamp = 123456789;

    let json = format!(
        concat!(
            "{{",
            " \"metadata\" :  {{",          // depth 1
              "   \"\"  :  \"{author}\"  ,",
              "   \"author\"  :  \"{author}\"  ,",
              "\"version\":\"{version}\",",
              "\"stats\":{{",           // depth 2
                "\"views\":{views},",
                " \"details\" :   {{  ",       // depth 3
                  "  \"regions\"   :   {regions},",
                  "  \"locale\"   :  \"{locale}\"  ",
                "}}",
              "}},",
              "\"name\": \"{name}\"",
            "}},",
            "\"\":{tags},",
            "\"items\":{items},",
            "\"config\":{{",            // depth 1
              "\"key\": \"{key}\",",
              "\"values\":{{",          // depth 2
                "\"value1\": \"{value1}\",",
                "\"value2\": \"{value2}\"",
              "}}",
            "}},",
            "\"signature\":{signature},",
            "\"timestamp\":{timestamp}",
            "}}"
        ),
        author   = &author,
        version  = &version,
        views    = &views,
        regions   = &regions,
        locale   = &locale,
        name   = &name,
        tags     = &tags,
        items    = &items,
        key  = &key,
        value1      = &value1,
        value2      = &value2,
        signature = &signature,
        timestamp = &timestamp,
    );
    all_expected.insert("metadata.".to_string(), to_json_value(&author));
    all_expected.insert("metadata.author".to_string(), to_json_value(&author));
    all_expected.insert("metadata.version".to_string(), to_json_value(&version));
    all_expected.insert("metadata.stats.views".to_string(), to_json_value(&views));
    all_expected.insert("metadata.stats.details.regions".to_string(), to_json_value(&regions));
    all_expected.insert("metadata.stats.details.locale".to_string(), to_json_value(&locale));
    all_expected.insert("metadata.name".to_string(), to_json_value(&name));
    all_expected.insert("".to_string(), to_json_value(&tags));
    all_expected.insert("items".to_string(), to_json_value(&items));
    all_expected.insert("config.key".to_string(), to_json_value(&key));
    all_expected.insert("config.values.value1".to_string(), to_json_value(&value1));
    all_expected.insert("config.values.value2".to_string(), to_json_value(&value2));
    all_expected.insert("signature".to_string(), to_json_value(&signature));
    all_expected.insert("timestamp".to_string(), to_json_value(&timestamp));


    let json_bytes = json.into_bytes();
    if verbose {
        println!("=== JSON structure ===");
        print_json_structure(&json_bytes);
        println!("=== end structure ===\n");
    }
    json_bytes
}

pub fn build_invalid_json(field_count: usize, value_len: usize, all_expected : &mut HashMap<String, Value>) -> Vec<u8> {
    let json: &mut Vec<u8> = &mut build_small_json(field_count, value_len, all_expected, true);
    json.append(&mut br#"-------"#.to_vec());
    json.append(&mut br#"+++++++"#.to_vec());
    json.to_vec()
}

pub fn build_text_input(field_count: usize, value_len: usize, mut all_expected : Option<&mut HashMap<String, Value>>) -> Vec<u8> {
    let mut text = build_text(field_count/3, value_len, ' ', " ", ' ', &mut all_expected).into_bytes();
    text.append(&mut br#"-------"#.to_vec());
    text.append(&mut build_text(field_count/3, value_len, ' ', " ", ' ', &mut all_expected).into_bytes());
    text.append(&mut br#"+++++++"#.to_vec());
    text.append(&mut build_text(field_count/3, value_len, ' ', " ", ' ', &mut all_expected).into_bytes());
    text
}

pub fn random_chunks(bytes: &Vec<u8>, min: usize, max: usize, seed: u64, split_random_keys: bool, keys: &Vec<&str>) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut pos = 0;
    let mut rng = seed;
    let mut last_split = false;
    while pos < bytes.len() {
        let range = (max - min) as u64;
        let size = min + (lcg_next(&mut rng) % range.max(1)) as usize;
        let end = (pos + size).min(bytes.len());
        let chunk = bytes[pos..end].to_vec();
        if split_random_keys {
            let mut did_split = false;
            if !last_split {
                last_split = true;
                let content = String::from_utf8_lossy(&chunk);
                for key in keys {
                    if let Some(key_start) = content.find(*key) {
                        // Split in the middle of the key so it straddles the chunk boundary,
                        // forcing the parser to handle keys split across chunk boundaries.
                        let mid = key_start + key.len() / 2 + 1;
                        if mid < chunk.len() {
                            chunks.push(chunk[..mid].to_vec());
                            chunks.push(chunk[mid..].to_vec());
                            did_split = true;
                            break;
                        }
                    }
                }
            } else {
                last_split = false
            }
            if !did_split {
                chunks.push(chunk);
            }
        } else {
            chunks.push(chunk);
        }
        pos = end;
    }
    chunks
}

pub fn to_json_value<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap_or_default()
}

pub fn bytes_to_value(b: &Vec<u8>) -> Value {
    return serde_json::from_slice::<Value>(&b).unwrap_or_default();
}

/// Walk `bytes` as JSON and return the value found at `path` as a String.
///
/// `path` is a sequence of object keys describing a nested location, e.g.
/// `&["metadata", "stats", "details", "region"]`.  A single-element path
/// `&["field_7"]` targets a top-level scalar.
///
/// - Scalar leaf (string / number / bool): returned as its text value.
/// - Object or array at the terminal key: the full raw JSON substring is returned.
/// - Arrays that do *not* appear as the terminal path component are skipped.
pub fn extract_json_value(bytes: &[u8], path: &[&str]) -> Value {
    let mut parser = JSONEventGenerator::new();
    if path.is_empty() {
        return json!(null);
    }
    let mut cursor = 0usize;
    let mut pending_key: Option<String> = None;
    // How many path components have been successfully entered.
    let mut matched_depth: usize = 0;
    // How many levels deep we are inside a subtree that is *not* on the path.
    let mut skipped_depth: usize = 0;
    // Non-zero while capturing a matched child object/array as raw JSON.
    let mut collecting_depth: usize = 0;
    let mut collect_start: usize = 0;

    loop {
        let JSONEventWrapper { consumed_bytes, event } =
            parser.next_event(&bytes[cursor..], true);
        cursor += consumed_bytes;
        match event {
            None if consumed_bytes == 0 => break, // true stall
            None => continue,
            Some(Err(_)) => break,
            Some(Ok(ev)) => match ev {
                JSONEvent::Eof => break,

                JSONEvent::ObjectKey(k) => {
                    if skipped_depth == 0 && collecting_depth == 0 {
                        pending_key = Some(k.to_string());
                    }
                }

                JSONEvent::StartObject => {
                    if collecting_depth > 0 {
                        // Inside an in-progress collection — just track nesting.
                        collecting_depth += 1;
                    } else {
                        let key = pending_key.take().unwrap_or_default();
                        if skipped_depth > 0 {
                            skipped_depth += 1;
                        } else if key.is_empty() {
                            // Root object — transparent, no path component consumed.
                        } else if matched_depth < path.len() && key == path[matched_depth] {
                            if matched_depth == path.len() - 1 {
                                // Path terminates here — collect the entire subtree as raw JSON.
                                // Use cursor-1 to point at the '{' itself, skipping any preceding
                                // ':' the parser consumed as part of this event.
                                collecting_depth = 1;
                                collect_start = cursor - 1;
                            } else {
                                // Navigate one level deeper along the path.
                                matched_depth += 1;
                            }
                        } else {
                            // Object not on the path — skip its entire subtree.
                            skipped_depth += 1;
                        }
                    }
                }

                JSONEvent::EndObject => {
                    if collecting_depth > 0 {
                        collecting_depth -= 1;
                        if collecting_depth == 0 {
                            return bytes_to_value(&bytes[collect_start..cursor].to_vec());
                        }
                    } else {
                        if skipped_depth > 0 {
                            skipped_depth -= 1;
                        } else if matched_depth > 0 {
                            matched_depth -= 1;
                        }
                        pending_key = None;
                    }
                }

                JSONEvent::StartArray => {
                    if collecting_depth > 0 {
                        // Nested array inside an in-progress collection.
                        collecting_depth += 1;
                    } else {
                        let key = pending_key.take();
                        if skipped_depth > 0 {
                            skipped_depth += 1;
                        } else if matched_depth < path.len()
                            && key.as_deref() == Some(path[matched_depth])
                            && matched_depth == path.len() - 1
                        {
                            // Path terminates at this array — collect it as raw JSON.
                            // Use cursor-1 to point at the '[' itself, skipping any preceding
                            // ':' the parser consumed as part of this event.
                            collecting_depth = 1;
                            collect_start = cursor - 1;
                        } else {
                            // Array not on the navigation path — skip its subtree.
                            skipped_depth += 1;
                        }
                    }
                }

                JSONEvent::EndArray => {
                    if collecting_depth > 0 {
                        collecting_depth -= 1;
                        if collecting_depth == 0 {
                            return bytes_to_value(&bytes[collect_start..cursor].to_vec());
                        }
                    } else if skipped_depth > 0 {
                        skipped_depth -= 1;
                    }
                }

                JSONEvent::String(val) => {
                    if collecting_depth == 0
                        && skipped_depth == 0
                        && matched_depth == path.len() - 1
                        && pending_key.as_deref() == Some(path[matched_depth])
                    {
                        return bytes_to_value(&serde_json::to_vec(&val.as_ref()).unwrap_or_default());
                    }
                    if collecting_depth == 0 {
                        pending_key = None;
                    }
                }

                JSONEvent::Number(val) => {
                    if collecting_depth == 0
                        && skipped_depth == 0
                        && matched_depth == path.len() - 1
                        && pending_key.as_deref() == Some(path[matched_depth])
                    {
                        return bytes_to_value(&val.into_owned().into_bytes());
                    }
                    if collecting_depth == 0 {
                        pending_key = None;
                    }
                }

                JSONEvent::Boolean(val) => {
                    if collecting_depth == 0
                        && skipped_depth == 0
                        && matched_depth == path.len() - 1
                        && pending_key.as_deref() == Some(path[matched_depth])
                    {
                        return bytes_to_value(&if val { b"true".to_vec() } else { b"false".to_vec() });
                    }
                    if collecting_depth == 0 {
                        pending_key = None;
                    }
                }

                _ => {
                    if collecting_depth == 0 {
                        pending_key = None;
                    }
                }
            },
        }
    }
    json!(null)
}

pub fn lcg_next(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state
}

/// Generate a repeating-char string of `len` bytes using character `ch`.
pub fn rep(ch: char, len: usize) -> String {
    std::iter::repeat(ch).take(len).collect()
}
