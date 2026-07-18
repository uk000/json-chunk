use std::collections::{HashMap};
use serde_json::{Value};
use json_chunk::chunk_parser::ChunkParser;

mod test_helpers;
use test_helpers::*;
mod print_helpers;
use print_helpers::*;

pub const MCP_METHOD_JSON_PATH: &str = r"method";
pub const MCP_TOOL_JSON_PATH: &str = r"params.name";

#[cfg(test)]
mod tests {

use super::*;

  #[test]
    fn test_detect_json_end_with_no_end_of_stream() {
        let mut all_expected : HashMap<String, Value> = HashMap::new();
        let json_bytes = build_invalid_json(3, 10, &mut all_expected);

        // Multiple target paths to find simultaneously during streaming.
        let path_map = HashMap::from([
            ("config.key".to_string(), (None, 100)),
            ("config.values.xxx".to_string(), (Some("value".to_string()), 0)),
        ]);

        let expected = build_expected(&path_map, &all_expected);
        let (all_names, successors) = get_expected_field_names(&path_map);
        // Split at random boundaries between 50 and 300 bytes (seed = 42).
        let chunks = random_chunks(&json_bytes, 50, 300, 42, true, &all_names);
        // print_relevant_chunks(&all_names, &successors, &chunks, 3);
        
        let total: usize = chunks.len();
        let mut parser = ChunkParser::new_json_parser(&path_map);
        for (i, chunk) in chunks.iter().enumerate() {
            parser.process_chunk(chunk, false);
            if parser.is_all_found() {
                break
            }
        }
        println!("json_depth = {}", parser.json_depth);
        println!("end_of_json = {}", parser.end_of_json);
        println!("short_circuit = {}", parser.short_circuit);
        println!("end_of_stream = {}", parser.end_of_stream);
        let json: Value = parser.get_result_json();
        print_jsons(&expected, &parser.matches_found, &json, false);
        assert_eq!(parser.is_all_found(), false);
    }

  #[test]
    fn test_short_circuit_early_finish() {
        let mut all_expected : HashMap<String, Value> = HashMap::new();
        let json_bytes = build_large_json(10, 10, &mut all_expected, false);

        // Multiple target paths to find simultaneously during streaming.
        let path_map: HashMap<String, (Option<String>, usize)> = HashMap::from([
            ("field_0".to_string(), (None, 100)),
        ]);
        test_happy_paths(&json_bytes, &path_map);

        // Multiple target paths to find simultaneously during streaming.
        let path_map: HashMap<String, (Option<String>, usize)> = HashMap::from([
            ("field_0".to_string(), (None, 100)),
            ("field_9".to_string(), (None, 100)),
        ]);
        test_happy_paths(&json_bytes, &path_map);

        // Multiple target paths to find simultaneously during streaming.
        let path_map: HashMap<String, (Option<String>, usize)> = HashMap::from([
            ("field_0".to_string(), (None, 100)),
            ("field_9".to_string(), (None, 100)),
            ("metadata.author".to_string(), (None, 100)),
        ]);
        test_happy_paths(&json_bytes, &path_map);
    }

    #[test]
    fn test_object_with_empty_fields() {
        let mut all_expected : HashMap<String, Value> = HashMap::new();
        let json_bytes = build_small_json(3, 10, &mut all_expected, false);

        // Multiple target paths to find simultaneously during streaming.
        let path_map = HashMap::from([
            ("metadata.".to_string(), (Some("metadata".to_string()), 100)),
            ("metadata.stats.details.locale".to_string(), (Some("locale".to_string()), 100)),
            ("config.key".to_string(), (None, 100)),
            ("config.values.value2".to_string(), (Some("value".to_string()), 0)),
            (".".to_string(), (Some("default".to_string()),100)),
        ]);

        let mut parser: ChunkParser = ChunkParser::new_json_parser(&path_map);

        let expected: HashMap<String, Value> = HashMap::from([
            ("metadata".to_string(), extract_json_value(&json_bytes, &["metadata",""])),
            ("locale".to_string(), extract_json_value(&json_bytes, &["metadata","stats","details","locale"])),
            ("config.key".to_string(), extract_json_value(&json_bytes, &["config","key"])),
            ("value".to_string(), extract_json_value(&json_bytes, &["config","values","value2"])),
            ("default".to_string(), extract_json_value(&json_bytes, &[""])),
        ]);

        let (all_names, successors) = get_expected_field_names(&path_map);
        // Split at random boundaries between 50 and 300 bytes (seed = 42).
        let chunks: Vec<Vec<u8>> = random_chunks(&json_bytes, 10, 50, 42, true, &all_names);
        // print_relevant_chunks(&all_names, &successors, &chunks, 3);
        
        feed_chunks_to_parser(&mut parser, &chunks, false);
        let json: Value = parser.get_result_json();
        print_jsons(&expected, &parser.matches_found, &json, false);
        assert_eq!(parser.is_all_found(), true);
        assert_eq!(parser.get_field("metadata.stats.details.locale").overflow, false);
        assert_eq!(parser.get_field("config.key").overflow, false);
        assert_eq!(parser.get_field("config.values.value2").overflow, false);
        assert_eq!(json, serde_json::to_value(expected).unwrap());
    }

    #[test]
    fn test_fields_overflow() {
        let mut all_expected : HashMap<String, Value> = HashMap::new();
        let json_bytes = build_large_json(3, 1000, &mut all_expected, false);

        // Multiple target paths to find simultaneously during streaming.
        let path_map = HashMap::from([
            ("field_1".to_string(), (Some("b".to_string()), 0)),
            ("metadata.stats.details.locale".to_string(), (Some("locale".to_string()), 200)),
            ("config.key".to_string(), (None, 100)),
            ("config.values.value2".to_string(), (Some("value".to_string()), 0)),
        ]);

        let (all_names, successors) = get_expected_field_names(&path_map);
        let chunks = random_chunks(&json_bytes, 50, 300, 42, true, &all_names);
        let mut expected = build_expected(&path_map, &all_expected);
        // Split at random boundaries between 50 and 300 bytes (seed = 42).
        // print_relevant_chunks(&all_names, &successors, &chunks, 3);
        
        let mut parser = ChunkParser::new_json_parser(&path_map);
        // print_relevant_chunks(&all_names, &successors, &chunks, 3);
        feed_chunks_to_parser(&mut parser, &chunks, false);
        let json: Value = parser.get_result_json();
        for k in &parser.overflowed_fields {
            let tracker = parser.get_field(k);
            expected.remove(k);
            if let Some(o) = &tracker.output_key {
                expected.remove(o);
            }
        }
        print_jsons(&expected, &parser.matches_found, &json, false);
        assert_eq!(parser.is_all_found(), true);
        assert_eq!(parser.get_field("field_1").overflow, false);
        assert_eq!(parser.get_field("metadata.stats.details.locale").overflow, true);
        assert_eq!(parser.get_field("config.key").overflow, true);
        assert_eq!(parser.get_field("config.values.value2").overflow, false);
        assert_eq!(json, serde_json::to_value(&expected).unwrap());
    }

    #[test]
    fn test_split_numeric_field() {
        let json = format!(
            concat!(
                "{{",
                "\"timestamp\":{timestamp}",
                "}}"
            ),
            timestamp = 123456789,
        );
        let json_bytes = json.into_bytes();
        println!("=== JSON ===");
        print_json_structure(&json_bytes);

        // Multiple target paths to find simultaneously during streaming.
        let path_map: HashMap<String, (Option<String>, usize)> = HashMap::from([
            ("timestamp".to_string(), (None,100)),
        ]);
        let mut expected : HashMap<String, Value> = HashMap::new();
        build_expected_kv("timestamp", 123456789, &mut expected);
        test_expected_happy_paths(&json_bytes, &path_map, &expected);
    }

    #[test]
    fn test_mix_flat_nested_fields_in_random_chunks() {
        let mut all_expected : HashMap<String, Value> = HashMap::new();
        let json_bytes = build_large_json(10, 50, &mut all_expected, false);

        // Multiple target paths to find simultaneously during streaming.
        let path_map = HashMap::from([
            ("field_1".to_string(), (Some("b".to_string()), 0)),
            ("metadata.stats.details.locale".to_string(), (Some("locale".to_string()), 100)),
            ("config.key".to_string(), (None, 100)),
            ("config.values.value2".to_string(), (Some("value".to_string()), 0)),
            ("timestamp".to_string(), (None,10)),
        ]);
        let expected = build_expected(&path_map, &all_expected);
        test_expected_happy_paths(&json_bytes, &path_map, &expected);
    }

    #[test]
    fn test_multi_nested_obj_arrays_in_random_chunks() {
        let mut all_expected : HashMap<String, Value> = HashMap::new();
        let json_bytes = build_large_json(2, 10, &mut all_expected, false);

        // Multiple target paths to find simultaneously during streaming.
        let path_map: HashMap<String, (Option<String>, usize)> = HashMap::from([
            ("metadata.stats.details.regions".to_string(), (Some("regions".to_string()), 0)),
            ("config.values".to_string(), (Some("values".to_string()), 0)),
            ("metadata.name".to_string(), (None, 0)),
        ]);
        test_happy_paths(&json_bytes, &path_map);
    }

    #[test]
    fn test_invalid_json_fields() {
        let mut all_expected : HashMap<String, Value> = HashMap::new();
        let json_bytes = build_large_json(20, 2_000, &mut all_expected, false);

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
        let chunks = random_chunks(&json_bytes, 50, 300, 42, false, &all_names);
        let mut parser = ChunkParser::new_json_parser(&path_map);
        let total: usize = chunks.len();
        for (i, chunk) in chunks.iter().enumerate() {
            parser.process_chunk(chunk, i == total - 1);
            if parser.is_all_found() {
                break
            }
        }
        let json: Value = parser.get_result_json();
        print_jsons(&expected, &parser.matches_found, &json, false);
        assert_eq!(parser.is_all_found(), false);
        assert_eq!(json, serde_json::to_value(expected).unwrap());
    }

  #[test]
    fn test_mix_valid_and_invalid_fields() {
        let mut all_expected : HashMap<String, Value> = HashMap::new();
        let json_bytes = build_large_json(10, 1000, &mut all_expected, false);

        // Multiple target paths to find simultaneously during streaming.
        let path_map = HashMap::from([
            ("field_x".to_string(), (Some("b".to_string()),1024)),
            ("metadata.stats.details.locale".to_string(), (Some("locale".to_string()),100)),
            ("config.stamp".to_string(), (None, 0)),
            ("timestamp".to_string(), (None, 0)),
        ]);

        let expected = build_expected(&path_map, &all_expected);
        let (all_names, successors) = get_expected_field_names(&path_map);
        // Split at random boundaries between 50 and 300 bytes (seed = 42).
        let chunks = random_chunks(&json_bytes, 50, 300, 42, true, &all_names);
        // print_relevant_chunks(&all_names, &successors, &chunks, 3);
        
        let total: usize = chunks.len();
        let mut parser = ChunkParser::new_json_parser(&path_map);
        for (i, chunk) in chunks.iter().enumerate() {
            parser.process_chunk(chunk, i == total - 1);
            if parser.is_all_found() {
                break
            }
        }
        let json: Value = parser.get_result_json();
        print_jsons(&expected, &parser.matches_found, &json, false);
        assert_eq!(parser.is_all_found(), false);
        assert_eq!(parser.get_field("metadata.stats.details.locale").overflow, true);
        assert_ne!(json, serde_json::to_value(expected).unwrap());
    }

  #[test]
    fn test_flat_text_fields() {
        let mut all_expected : HashMap<String, Value> = HashMap::new();
        let text_bytes = build_text_input(30, 10, Some(&mut all_expected));
        let path_map: HashMap<String, (Option<String>, usize)> = HashMap::from([
            ("field_0".to_string(), (None, 0)),
            ("field_2".to_string(), (None, 0)),
            ("field_5".to_string(), (None, 0)),
        ]);
        let (all_names, successors) = get_expected_field_names(&path_map);
        let chunks = random_chunks(&text_bytes, 50, 300, 42, true, &all_names);
        // print_relevant_chunks(&all_names, &successors, &chunks, 3);
        let total: usize = chunks.len();
        let mut parser = ChunkParser::new_json_parser(&path_map);
        for (i, chunk) in chunks.iter().enumerate() {
            parser.process_chunk(chunk, i == total - 1);
            if parser.is_all_found() {
                break
            }
        }
        let json: Value = parser.get_result_json();
        let expected : HashMap<String, Value> = HashMap::new();
        print_jsons(&expected, &parser.matches_found, &json, false);
    }
}

pub fn test_happy_paths(json_bytes: &Vec<u8>, path_map: &HashMap<String, (Option<String>, usize)>) {
    let mut parser = ChunkParser::new_json_parser(path_map);
    let (all_names, successors) = get_expected_field_names(&path_map);
    let chunks: Vec<Vec<u8>> = random_chunks(&json_bytes, 10, 50, 42, true, &all_names);
    let expected = build_expected_with_pos(&path_map, &chunks);
    // print_relevant_chunks(&all_names, &successors, &chunks, 3);
    let last_chunk_o = feed_chunks_to_parser(&mut parser, &chunks, false);
    let matches = parser.get_matches().clone();
    let json: Value = parser.get_result_json();
    print_jsons_with_chunk_info(&expected, &matches, &json, false);
    assert_eq!(parser.is_all_found(), true);
    assert_eq!(last_chunk_o.is_some(), true);
    let actual_last_chunk = last_chunk_o.unwrap();
    println!("Parser exited at chunk {} with {} remaining", actual_last_chunk, chunks.len()-actual_last_chunk);
    let mut expected_last_chunk = 0;
    for (key, (out_key, _)) in path_map {
        let mut field = key;
        if let Some(v) = out_key {
            field = v;
        }
        let field_chunk = expected.get(field).unwrap().0;
        assert_eq!(field_chunk <= actual_last_chunk, true);
        if expected_last_chunk < field_chunk {
            expected_last_chunk = field_chunk;
        }
    }
    assert_eq!(expected_last_chunk, actual_last_chunk);
}

pub fn test_expected_happy_paths(json_bytes: &Vec<u8>, path_map: &HashMap<String, (Option<String>, usize)>, expected : &HashMap<String, Value>) {
    let mut parser = ChunkParser::new_json_parser(path_map);
    let (all_names, successors) = get_expected_field_names(&path_map);
    let chunks: Vec<Vec<u8>> = random_chunks(&json_bytes, 10, 50, 42, true, &all_names);
    // print_relevant_chunks(&all_names, &successors, &chunks, 3);
    let last_chunk_o = feed_chunks_to_parser(&mut parser, &chunks, false);
    let matches = parser.get_matches().clone();
    let json: Value = parser.get_result_json();
    print_jsons(&expected, &matches, &json, false);
    assert_eq!(parser.is_all_found(), true);
    assert_eq!(last_chunk_o.is_some(), true);
    let actual_last_chunk = last_chunk_o.unwrap();
    println!("Parser exited at chunk {} with {} remaining", actual_last_chunk, chunks.len()-actual_last_chunk);
}
