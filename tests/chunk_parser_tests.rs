use std::collections::{HashMap};
use std::sync::{LazyLock, Mutex};
use serde_json::{json, Value};
use json_chunk::parser::{JSONEvent, JSONEventGenerator, JSONEventWrapper};


static SUCCESSORS: LazyLock<Mutex<HashMap<&'static str, &'static str>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(test)]
mod tests {

use json_chunk::chunk_parser::JSONChunkParser;

use super::*;

  fn create_parser(paths: HashMap<String, (Option<String>, usize)>) -> JSONChunkParser {
    let mut parser = JSONChunkParser::default();
    for (path, value) in paths {
      parser.add_search_field(path, value.0, value.1);
    }
    parser
  }

  #[test]
  fn test_object_with_empty_fields() {
    let json_bytes = build_small_json(3, 10);

    // Multiple target paths to find simultaneously during streaming.
    let path_map = HashMap::from([
        ("metadata.".to_string(), (Some("metadata".to_string()), 100)),
        ("metadata.stats.details.locale".to_string(), (Some("locale".to_string()), 100)),
        ("config.key".to_string(), (None, 100)),
        ("config.values.value2".to_string(), (Some("value".to_string()), 0)),
        ("timestamp".to_string(), (None,10)),
        (".".to_string(), (Some("default".to_string()),100)),
    ]);

    let expected: HashMap<String, Value> = HashMap::from([
        ("metadata".to_string(), extract_json_value(&json_bytes, &["metadata",""])),
        ("locale".to_string(), extract_json_value(&json_bytes, &["metadata","stats","details","locale"])),
        ("config.key".to_string(), extract_json_value(&json_bytes, &["config","key"])),
        ("value".to_string(), extract_json_value(&json_bytes, &["config","values","value2"])),
        ("timestamp".to_string(), extract_json_value(&json_bytes, &["timestamp"])),
        ("default".to_string(), extract_json_value(&json_bytes, &[""])),
    ]);

    let (all_names, successors) = get_expected_field_names(&path_map);
    // Split at random boundaries between 50 and 300 bytes (seed = 42).
    let chunks = random_chunks(json_bytes, 10, 50, 42, true, &all_names);
    print_relevant_chunks(&all_names, &successors, &chunks);
    
    let total: usize = chunks.len();
    let mut parser = create_parser(path_map);
    for (i, chunk) in chunks.iter().enumerate() {
        println!("Processing chunk {}/{} bytes {}", i+1, chunks.len(), chunk.len());
        parser.process_chunk(chunk, i == total - 1);
        if parser.is_all_found() {
            break
        }
    }
    println!("expected ({}):", expected.len());
    print_mapjson_summary(&expected, false);
    println!("parser.matches_found ({}):", parser.matches_found.len());
    print_mapjson_summary(&parser.matches_found, false);
    assert_eq!(parser.is_all_found(), true);
    assert_eq!(parser.get_field("metadata.stats.details.locale").overflow, false);
    assert_eq!(parser.get_field("config.key").overflow, false);
    assert_eq!(parser.get_field("config.values.value2").overflow, false);
    assert_eq!(parser.get_field("timestamp").overflow, false);
    let json = parser.get_result_json();
    println!("Result JSON:");
    print_json_summary(&json, false);
    assert_eq!(json, serde_json::to_value(expected).unwrap());
  
  }

  #[test]
  fn test_fields_overflow() {
    let json_bytes = build_large_json(3, 1000);

    // Multiple target paths to find simultaneously during streaming.
    let path_map = HashMap::from([
        ("field_1".to_string(), (Some("b".to_string()), 0)),
        ("metadata.stats.details.locale".to_string(), (Some("locale".to_string()), 200)),
        ("config.key".to_string(), (None, 100)),
        ("config.values.value2".to_string(), (Some("value".to_string()), 0)),
        ("timestamp".to_string(), (None,10)),
    ]);

    let expected = build_expected(&path_map, &json_bytes);
    let (all_names, successors) = get_expected_field_names(&path_map);
    // Split at random boundaries between 50 and 300 bytes (seed = 42).
    let chunks = random_chunks(json_bytes, 50, 300, 42, true, &all_names);
    print_relevant_chunks(&all_names, &successors, &chunks);
    
    let total: usize = chunks.len();
    let mut parser = create_parser(path_map);
    for (i, chunk) in chunks.iter().enumerate() {
        println!("Processing chunk {}/{} bytes {}", i+1, total, chunk.len());
        parser.process_chunk(chunk, i == total - 1);
        if parser.is_all_found() {
            break
        }
    }
    println!("expected ({}):", expected.len());
    print_mapjson_summary(&expected, false);
    println!("parser.matches_found ({}):", parser.matches_found.len());
    print_mapjson_summary(&parser.matches_found, false);
    assert_eq!(parser.is_all_found(), false);
    assert_eq!(parser.get_field("field_1").overflow, false);
    assert_eq!(parser.get_field("metadata.stats.details.locale").overflow, true);
    assert_eq!(parser.get_field("config.key").overflow, true);
    assert_eq!(parser.get_field("config.values.value2").overflow, false);
    assert_eq!(parser.get_field("timestamp").overflow, false);
    let json = parser.get_result_json();
    println!("Result JSON:");
    print_json_summary(&json, false);
    assert_ne!(json, serde_json::to_value(expected).unwrap());
  }

  #[test]
  fn test_mix_flat_nested_fields_in_random_chunks() {
    let json_bytes = build_large_json(10, 50);

    // Multiple target paths to find simultaneously during streaming.
    let path_map = HashMap::from([
        ("field_1".to_string(), (Some("b".to_string()), 0)),
        ("metadata.stats.details.locale".to_string(), (Some("locale".to_string()), 100)),
        ("config.key".to_string(), (None, 100)),
        ("config.values.value2".to_string(), (Some("value".to_string()), 0)),
        ("timestamp".to_string(), (None,10)),
    ]);

    let expected = build_expected(&path_map, &json_bytes);
    let (all_names, successors) = get_expected_field_names(&path_map);
    // Split at random boundaries between 50 and 300 bytes (seed = 42).
    let chunks = random_chunks(json_bytes, 50, 300, 42, true, &all_names);
    print_relevant_chunks(&all_names, &successors, &chunks);
    
    let total: usize = chunks.len();
    let mut parser = create_parser(path_map);
    for (i, chunk) in chunks.iter().enumerate() {
        println!("Processing chunk {}/{} bytes {}", i+1, total, chunk.len());
        parser.process_chunk(chunk, i == total - 1);
        if parser.is_all_found() {
            break
        }
    }
    println!("expected ({}):", expected.len());
    print_mapjson_summary(&expected, false);
    println!("parser.matches_found ({}):", parser.matches_found.len());
    print_mapjson_summary(&parser.matches_found, false);
    assert_eq!(parser.is_all_found(), true);
    let json = parser.get_result_json();
    println!("Result JSON:");
    print_json_summary(&json, false);
    assert_eq!(json, serde_json::to_value(expected).unwrap());
  }

  #[test]
  fn test_multi_nested_obj_arrays_in_random_chunks() {
    let json_bytes = build_large_json(2, 10);

    // Multiple target paths to find simultaneously during streaming.
    let path_map: HashMap<String, (Option<String>, usize)> = HashMap::from([
        ("metadata.stats.details.regions".to_string(), (Some("regions".to_string()), 0)),
        ("config.values".to_string(), (Some("values".to_string()), 0)),
        ("metadata.name".to_string(), (None, 0)),
    ]);

    let expected = build_expected(&path_map, &json_bytes);
    let (all_names, successors) = get_expected_field_names(&path_map);
    // Split at random boundaries between 50 and 300 bytes (seed = 42).
    let chunks = random_chunks(json_bytes, 50, 300, 42, true, &all_names);
    print_relevant_chunks(&all_names, &successors, &chunks);
    
    let total: usize = chunks.len();
    let mut parser = create_parser(path_map);
    for (i, chunk) in chunks.iter().enumerate() {
        println!("Processing chunk {}/{} bytes {}", i+1, total, chunk.len());
        parser.process_chunk(chunk, i == total - 1);
        if parser.is_all_found() {
            break
        }
    }
    println!("expected ({}):", expected.len());
    print_mapjson_summary(&expected, true);
    println!("parser.matches_found ({}):", parser.matches_found.len());
    print_mapjson_summary(&parser.matches_found, true);
    assert_eq!(parser.is_all_found(), true);
    let json = parser.get_result_json();
    println!("Result JSON:");
    print_json_summary(&json, true);
    assert_eq!(json, serde_json::to_value(expected).unwrap());
  }

  #[test]
  fn test_invalid_json_fields() {
    let json_bytes = build_large_json(20, 2_000);

    // Multiple target paths to find simultaneously during streaming.
    let path_map = HashMap::from([
        ("field_x".to_string(), (None, 0)),
        ("metadata.foo.details.region".to_string(), (Some("region".to_string()), 512)),
        ("foo.name".to_string(), (None, 256)),
    ]);
    //nothing expected, so empty map
    let expected : HashMap<String, Value> = HashMap::new();
    let all_names: Vec<&str> = vec![];
    // Split at random boundaries between 50 and 300 bytes (seed = 42).
    let chunks = random_chunks(json_bytes, 50, 300, 42, false, &all_names);
    let mut parser = create_parser(path_map);
    let total: usize = chunks.len();
    for (i, chunk) in chunks.iter().enumerate() {
        println!("Processing chunk {}/{} bytes {}", i+1, total, chunk.len());
        parser.process_chunk(chunk, i == total - 1);
        if parser.is_all_found() {
            break
        }
    }
    println!("expected ({}):", expected.len());
    print_mapjson_summary(&expected, false);
    println!("parser.matches_found ({}):", parser.matches_found.len());
    print_mapjson_summary(&parser.matches_found, false);
    assert_eq!(parser.is_all_found(), false);
    let json = parser.get_result_json();
    println!("Result JSON:");
    print_json_summary(&json, false);
    assert_eq!(json, serde_json::to_value(expected).unwrap());
  }

  #[test]
  fn test_mix_valid_and_invalid_fields() {
    let json_bytes = build_large_json(10, 1000);

    // Multiple target paths to find simultaneously during streaming.
    let path_map = HashMap::from([
        ("field_x".to_string(), (Some("b".to_string()),1024)),
        ("metadata.stats.details.locale".to_string(), (Some("locale".to_string()),100)),
        ("config.stamp".to_string(), (None, 0)),
        ("timestamp".to_string(), (None, 0)),
    ]);

    let expected = build_expected(&path_map, &json_bytes);
    let (all_names, successors) = get_expected_field_names(&path_map);
    // Split at random boundaries between 50 and 300 bytes (seed = 42).
    let chunks = random_chunks(json_bytes, 50, 300, 42, true, &all_names);
    print_relevant_chunks(&all_names, &successors, &chunks);
    
    let total: usize = chunks.len();
    let mut parser = create_parser(path_map);
    for (i, chunk) in chunks.iter().enumerate() {
        println!("Processing chunk {}/{} bytes {}", i+1, total, chunk.len());
        parser.process_chunk(chunk, i == total - 1);
        if parser.is_all_found() {
            break
        }
    }
    println!("expected ({}):", expected.len());
    print_mapjson_summary(&expected, false);
    println!("parser.matches_found ({}):", parser.matches_found.len());
    print_mapjson_summary(&parser.matches_found, false);
    assert_eq!(parser.is_all_found(), false);
    assert_eq!(parser.get_field("metadata.stats.details.locale").overflow, true);
    let json = parser.get_result_json();
    println!("Result JSON:");
    print_json_summary(&json, false);
    assert_ne!(json, serde_json::to_value(expected).unwrap());
  }
}

fn v(ch: char, value_len: usize) -> String { 
    format!("\"{}\"", rep(ch, value_len))
}

fn build_flat_fields(field_count: usize, value_len: usize) -> String {
    let mut flat_fields = String::new();
    for i in 0..field_count {
        let ch = (b'a' + (i % 26) as u8) as char;
        flat_fields.push_str(&format!("\"field_{i}\":{}", v(ch, value_len)));
        flat_fields.push(',');
    }
    flat_fields
}

fn str_array(ch: char, item_count: usize, value_len: usize) -> String {
    let items: Vec<String> = (0..item_count).map(|_| v(ch, value_len)).collect();
    format!("[ {} ]", items.join(","))
}

fn obj_array(kch: char, vch: char, item_count: usize, value_len: usize) -> String {
    let items: Vec<String> = (0..item_count)
        .map(|_| format!("{{\"name\":{},\"value\":{}}}", v(kch, value_len), v(vch, value_len)))
        .collect();
    format!("[ {} ]", items.join(","))
}

fn set_larger_json_successors() {
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

// ── Test helpers ─────────────────────────────────────────────────────────────
/// Build a large JSON object that mixes:
/// - `field_count` flat scalar string fields (`field_0` … `field_{N-1}`)
/// - A nested `metadata` object (3 levels deep)
/// - A `tags` array of strings
/// - An `items` array of objects
/// - A `config` nested object (2 levels deep)
/// - A `keywords` array of strings
///
/// `field_count` also controls how many elements the arrays contain.
/// `value_len` sets the length of every string value.
fn build_large_json(field_count: usize, value_len: usize) -> Vec<u8> {
    // ── flat scalar fields: field_0 … field_{N-1} ───────────────────────────
    let flat_fields = build_flat_fields(field_count, value_len);
    // flat_fields always ends with ',' — the nested section follows
    set_larger_json_successors();

    let json = format!(
        concat!(
            "{{",
            "{flat}",                   // field_0 … field_{N-1} (each ending with ',')
            "\"metadata\" : {{",          // depth 1
              "\"author\" :  {author},",
              "\"version\" :  {version},",
              " \"stats\" :  {{",           // depth 2
                "  \"views\":{views},",
                "  \"details\":{{",       // depth 3
                  "  \"regions\":{regions},",
                  "  \"locale\":{locale}",
                "}}",
              "}},",
              "\"name\":{name}",
            "}},",
            "\"tags\":{tags},",
            "\"items\":{items},",
            "\"config\":{{",            // depth 1
              "\"key\":{key},",
              "\"values\":{{",          // depth 2
                "\"value1\":{value1},",
                "\"value2\":{value2}",
              "}}",
            "}},",
            "\"signature\":{signature},",
            "\"timestamp\":{timestamp}",
            "}}"
        ),
        flat     = flat_fields,
        author   = v('A', value_len),
        version  = v('B', value_len),
        views    = obj_array('C', 'D', field_count, value_len),
        regions   = str_array('E', field_count, value_len),
        locale   = v('F', value_len),
        name   = v('G', value_len),
        tags     = str_array('H', field_count, value_len),
        items    = obj_array('I', 'J', field_count, value_len),
        key  = v('K', value_len),
        value1      = v('L', value_len),
        value2      = v('M', value_len),
        signature = str_array('N', field_count, value_len),
        timestamp = 123456,
    );
    println!("=== JSON structure ===");
    let json_bytes = json.into_bytes();
    print_json_structure(&json_bytes);
    println!("=== end structure ===\n");
    json_bytes
}

fn build_small_json(field_count: usize, value_len: usize) -> Vec<u8> {
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

    let json = format!(
        concat!(
            "{{",
            " \"metadata\" :  {{",          // depth 1
              "   \"\"  :   {author}  ,",
              "\"version\":{version},",
              "\"stats\":{{",           // depth 2
                "\"views\":{views},",
                " \"details\" :   {{  ",       // depth 3
                  "  \"regions\"   :   {regions},",
                  "  \"locale\"   :  {locale}  ",
                "}}",
              "}},",
              "\"name\":{name}",
            "}},",
            "\"\":{tags},",
            "\"items\":{items},",
            "\"config\":{{",            // depth 1
              "\"key\":{key},",
              "\"values\":{{",          // depth 2
                "\"value1\":{value1},",
                "\"value2\":{value2}",
              "}}",
            "}},",
            "\"signature\":{signature},",
            "\"timestamp\":{timestamp}",
            "}}"
        ),
        author   = v('A', value_len),
        version  = v('B', value_len),
        views    = obj_array('C', 'D', field_count, value_len),
        regions   = str_array('E', field_count, value_len),
        locale   = v('F', value_len),
        name   = v('G', value_len),
        tags     = str_array('H', field_count, value_len),
        items    = obj_array('I', 'J', field_count, value_len),
        key  = v('K', value_len),
        value1      = v('L', value_len),
        value2      = v('M', value_len),
        signature = str_array('N', field_count, value_len),
        timestamp = 123456,
    );
    println!("=== JSON structure ===");
    let json_bytes = json.into_bytes();
    print_json_structure(&json_bytes);
    println!("=== end structure ===\n");
    json_bytes
}

/// Split `bytes` at random positions (chunk size between `min` and `max` bytes).
fn random_chunks(bytes: Vec<u8>, min: usize, max: usize, seed: u64, split_random_keys: bool, keys: &Vec<&str>) -> Vec<Vec<u8>> {
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

fn bytes_to_value(b: &Vec<u8>) -> Value {
    return serde_json::from_slice::<Value>(&b).unwrap_or_default();
}

fn build_expected(path_map: &HashMap<String, (Option<String>, usize)>, json_bytes: &Vec<u8>) -> HashMap<String, Value> {
    let mut expected : HashMap<String, Value> = HashMap::new();
    // Derive expected values by navigating each path in the full JSON.
    for (json_path, output) in path_map {
        let fields_vec: Vec<&str> = json_path.split('.').collect();
        let json_value = extract_json_value(&json_bytes, &fields_vec);
        if !json_value.is_null() {
            expected.insert(output.clone().0.unwrap_or(json_path.to_string()), json_value);
        }
    }
    expected
}

fn get_expected_field_names(path_map: &HashMap<String, (Option<String>, usize)>) -> (Vec<&str>, HashMap<&str, &str>) {
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

/// Walk `bytes` as JSON and return the value found at `path` as a String.
///
/// `path` is a sequence of object keys describing a nested location, e.g.
/// `&["metadata", "stats", "details", "region"]`.  A single-element path
/// `&["field_7"]` targets a top-level scalar.
///
/// - Scalar leaf (string / number / bool): returned as its text value.
/// - Object or array at the terminal key: the full raw JSON substring is returned.
/// - Arrays that do *not* appear as the terminal path component are skipped.
fn extract_json_value(bytes: &[u8], path: &[&str]) -> Value {
    if path.is_empty() {
        return json!(null);
    }
    let mut parser = JSONEventGenerator::new();
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
            parser.parse_next(&bytes[cursor..], true);
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

/// Minimal LCG so we don't need the `rand` crate.
/// Walk `bytes` as JSON and print its structure to stdout.
/// String values are summarised as:
///   `string of N chars starting with "XYZ" and ending with "XYZ"`
/// so that very long values don't flood the terminal.
fn print_json_structure(bytes: &[u8]) {
    // Each stack entry: (is_array, next_array_index)
    let mut stack: Vec<(bool, usize)> = Vec::new();
    let mut pending_key: Option<String> = None;
    let mut parser = JSONEventGenerator::new();
    let mut cursor = 0usize;

    // Build the indentation string for the current depth.
    let ind = |depth: usize| "  ".repeat(depth);

    // Return (and consume) the label for the current position.
    // If we have a pending object key, use it.
    // If we're directly inside an array, use the next index.
    // Otherwise (root) return an empty string.
    let take_label = |pending_key: &mut Option<String>,
                      stack: &mut Vec<(bool, usize)>|
     -> String {
        if let Some(mut k) = pending_key.take() {
            if k == "" {
                k = "\"\"".to_string();
            }
            format!("{k}: ")
        } else if let Some((true, idx)) = stack.last_mut() {
            let i = *idx;
            *idx += 1;
            format!("[{i}]: ")
        } else {
            String::new()
        }
    };

    loop {
        let JSONEventWrapper { consumed_bytes, event } =
            parser.parse_next(&bytes[cursor..], true);
        cursor += consumed_bytes;

        match event {
            None if consumed_bytes == 0 => break,
            None => continue,
            Some(Err(_)) => break,
            Some(Ok(ev)) => match ev {
                JSONEvent::Eof => break,

                JSONEvent::ObjectKey(k) => {
                    pending_key = Some(k.to_string());
                }

                JSONEvent::StartObject => {
                    let label = take_label(&mut pending_key, &mut stack);
                    println!("{}{label}{{", ind(stack.len()));
                    stack.push((false, 0));
                }

                JSONEvent::EndObject => {
                    stack.pop();
                    println!("{}}}", ind(stack.len()));
                }

                JSONEvent::StartArray => {
                    let label = take_label(&mut pending_key, &mut stack);
                    println!("{}{label}[", ind(stack.len()));
                    stack.push((true, 0));
                }

                JSONEvent::EndArray => {
                    stack.pop();
                    println!("{}]", ind(stack.len()));
                }

                JSONEvent::String(val) => {
                    let label = take_label(&mut pending_key, &mut stack);
                    let s = val.as_ref();
                    let len = s.len();
                    let start: String = s.chars().take(3).collect();
                    let end: String = {
                        let chars: Vec<char> = s.chars().collect();
                        let n = chars.len().min(3);
                        chars[chars.len() - n..].iter().collect()
                    };
                    println!("{}{label}\"{start}...{len}...{end}\"", ind(stack.len()));
                }

                JSONEvent::Number(val) => {
                    let label = take_label(&mut pending_key, &mut stack);
                    println!("{}{label}(number) {val}", ind(stack.len()));
                }

                JSONEvent::Boolean(val) => {
                    let label = take_label(&mut pending_key, &mut stack);
                    println!("{}{label}(bool) {val}", ind(stack.len()));
                }

                JSONEvent::Null => {
                    let label = take_label(&mut pending_key, &mut stack);
                    println!("{}{label}null", ind(stack.len()));
                }
            },
        }
    }
}

fn find_and_print_key_in_chunk(key: &str, chunk: &Vec<u8>, next_chunk: &Vec<u8>, chunk_idx: usize, overlap: bool, prefix: &str) -> bool {
    if key == "" {
        return false;
    }
    let mut field = "field";
    if prefix != "" {
        field = prefix;
    }
    let content = String::from_utf8_lossy(chunk);
    let mut search_content = content.clone();
    if let Some(s) = search_content.find(key) {
        let end = (s + key.len()+10).min(content.len());
        // fixed width columns: | prefix | field | chunk_idx | chunk_len | snippet
        println!("| {:<15} | {:<20} | {:>15} | {:>15} | ```{}```",
            field, key, chunk_idx, "", &content[..end]);
        return true;
    }
    if overlap {
        let mut joined = Vec::with_capacity(20.min(chunk.len()) + next_chunk.len());
        let overlap_end = (key.len() + 20).min(next_chunk.len()); 
        let next_content = String::from_utf8_lossy(&next_chunk[..overlap_end]);
        joined.extend_from_slice(&chunk);
        joined.extend_from_slice(&next_chunk[..overlap_end]);
        search_content = String::from_utf8_lossy(&joined);
        if let Some(_) = search_content.find(key) {
            // across chunks: show both parts in fixed columns
            println!("| {:<15} | {:<20} | {:>15} | {:>15} | ```{}```  >>>>  ```{}```",
                field, key, chunk_idx, chunk_idx+1, &content, &next_content);
            return true;
        }
    }
    false
}

fn find_and_print_in_single_or_overlap(key: &str, chunks: &Vec<Vec<u8>>, chunk_idx: usize, total: usize, prefix: &str) -> bool {
  let chunk = &chunks[chunk_idx];
  let mut next_chunk : &Vec<u8> = &Vec::new();
  if chunk_idx + 1 < total {
    next_chunk = &chunks[chunk_idx+1];
  }
  if find_and_print_key_in_chunk(key, &chunks[chunk_idx], &next_chunk, chunk_idx, false, prefix) {
    return true;
  } else if chunk_idx + 1 < total {
    if find_and_print_key_in_chunk(key, &chunk, &next_chunk, chunk_idx+1, true, prefix) {
      return true;
    }
  }
  false
}

fn print_relevant_chunks(all_names: &Vec<&str>, successors: &HashMap<&str, &str>, chunks: &Vec<Vec<u8>>) {
    let total: usize = chunks.len();
    println!("\nOut of {} chunks, showing only chunks containing target fields {:?}:", total, all_names);

    let mut matched_successors: Vec<&&str> = Vec::new();
    println!("| {:<15} | {:<20} | {:>15} | {:>15} | {}",
            "kind", "field name", "from chunk", "split to ", "chunk");
    for (i, _) in chunks.iter().enumerate() {
      for name in all_names.iter() {
        if find_and_print_in_single_or_overlap(name, chunks, i, total, "") {
          matched_successors.push(successors.get(name).unwrap_or(&""));
        }
      }
      for name in matched_successors.clone().iter() {
        if find_and_print_in_single_or_overlap(name, chunks, i, total, "successor") {
          matched_successors.remove(0);
        }
      }
    }
    println!();
}

fn print_kv(k: &String, v: &Value, all: bool) {
    let value = serde_json::to_string(&v).unwrap();
    let len = value.len()-2;
    if all || len < 10 {
        println!("  {k}: {value}");
        return;
    }
    let start: String = value.chars().take(4).collect();
    let end: String = {
        let chars: Vec<char> = value.chars().collect();
        let n = chars.len().min(4);
        chars[chars.len() - n..].iter().collect()
    };
    println!("  {k}: {start}...{len}...{end}");
}

fn print_mapjson_summary(json : &HashMap<String, Value>, all: bool) {
    let empty: String = "\"\"".to_string();
    println!("{{");
    for (mut k, v) in json.iter() {
        if k == "" {
            k = &empty;
        }
        print_kv(k, v, all);
    }
    println!("}}");
}

fn print_json_summary(json : &Value, all: bool) {
    println!("{{");
    if let Value::Object(map) = json {
        for (key, value) in map {
            print_kv(key, &value, all);
        }
    } else {
        println!("not an object: {}", json);
    }
    println!("}}");
}

fn lcg_next(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state
}

/// Generate a repeating-char string of `len` bytes using character `ch`.
fn rep(ch: char, len: usize) -> String {
    std::iter::repeat(ch).take(len).collect()
}
