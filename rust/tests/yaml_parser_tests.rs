// ─── Tests ─────────────────────────────────────────────────────────────────────

use std::borrow::Cow;

use json_chunk::yaml_parser::YAMLEvent;

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, collections::HashMap};

use json_chunk::{ChunkParser, yaml_parser::{YAMLEvent, YAMLEventGenerator, is_yaml_number}};

use super::*;

    fn collect_events(input: &str) -> Vec<YAMLEvent<'static>> {
        let mut parser = YAMLEventGenerator::new();
        let buf = input.as_bytes();
        let mut events = Vec::new();
        loop {
            let wrap = parser.next_event(buf, true);
            let Some(result) = wrap.event else { break };
            let ev = result.expect("parse error");
            let done = ev == YAMLEvent::Eof;
            events.push(owned_yaml_event(ev));
            if done {
                break;
            }
        }
        events
    }
    fn path_map(paths: &[(&str, usize)]) -> HashMap<String, (Option<String>, usize)> {
        paths
            .iter()
            .map(|(p, sz)| (p.to_string(), (None, *sz)))
            .collect()
    }

    #[test]
    fn test_json_process_chunk_scalar() {
        let mut cp = ChunkParser::new_json_parser(&path_map(&[("a.b", 256)]));
        cp.process_chunk(&b"{\"a\": {\"b\": 42}}".to_vec(), true);
        let v = cp.matches_found.get("a.b").expect("a.b not found");
        assert_eq!(v, &serde_json::json!(42));
    }

    #[test]
    fn test_yaml_process_chunk_nested_scalar() {
        // Basic nested mapping: a.b should resolve to 42.
        let yaml = b"a:\n  b: 42\n";
        let mut cp = ChunkParser::new_yaml_parser(&path_map(&[("a.b", 256)]));
        cp.process_chunk(&yaml.to_vec(), true);
        let v = cp.matches_found.get("a.b").expect("a.b not found in YAML result");
        assert_eq!(v, &serde_json::json!(42));
    }

    #[test]
    fn test_yaml_process_chunk_string_value() {
        let yaml = b"name: hello\n";
        let mut cp = ChunkParser::new_yaml_parser(&path_map(&[("name", 256)]));
        cp.process_chunk(&yaml.to_vec(), true);
        let v = cp.matches_found.get("name").expect("name not found");
        assert_eq!(v, &serde_json::json!("hello"));
    }

    #[test]
    fn test_yaml_process_chunk_flow_nested() {
        // Flow YAML: same structural logic as JSON.
        let yaml = b"{a: {b: 3}}\n";
        let mut cp = ChunkParser::new_yaml_parser(&path_map(&[("a.b", 256)]));
        cp.process_chunk(&yaml.to_vec(), true);
        let v = cp.matches_found.get("a.b").expect("a.b not found");
        assert_eq!(v, &serde_json::json!(3));
    }

    #[test]
    fn test_yaml_chunked_input() {
        // Split across two chunks to verify streaming correctness.
        let chunk1 = b"a:\n  b: ".to_vec();
        let chunk2 = b"99\n".to_vec();
        let mut cp = ChunkParser::new_yaml_parser(&path_map(&[("a.b", 256)]));
        cp.process_chunk(&chunk1, false);
        cp.process_chunk(&chunk2, true);
        let v = cp.matches_found.get("a.b").expect("a.b not found");
        assert_eq!(v, &serde_json::json!(99));
    }

    #[test]
    fn test_typed_mapping() {
        // n: 42 should emit Number, not String
        let events = collect_events("n: 42\nb: true\ns: hello\nnil: null\n");
        assert!(events.contains(&YAMLEvent::Number(Cow::Borrowed("42"))));
        assert!(events.contains(&YAMLEvent::Boolean(true)));
        assert!(events.contains(&YAMLEvent::String(Cow::Borrowed("hello"))));
        assert!(events.contains(&YAMLEvent::Null));
    }

    #[test]
    fn test_nested_block() {
        let yaml = "outer:\n  inner: value\n";
        let events = collect_events(yaml);
        // Should contain: StreamStart, DocumentStart, StartObject,
        //   ObjectKey("outer"), StartObject, ObjectKey("inner"), String("value"),
        //   EndObject, EndObject, DocumentEnd, StreamEnd, Eof
        assert!(events.contains(&YAMLEvent::StartObject));
        assert!(events.contains(&YAMLEvent::ObjectKey(Cow::Borrowed("outer"))));
        assert!(events.contains(&YAMLEvent::ObjectKey(Cow::Borrowed("inner"))));
        assert!(events.contains(&YAMLEvent::String(Cow::Borrowed("value"))));
        assert_eq!(
            events.iter().filter(|e| **e == YAMLEvent::EndObject).count(),
            2
        );
    }

    #[test]
    fn test_flow_nested() {
        // {a: [1, 2], b: 3} must not break after `]`
        let events = collect_events("{a: [1, 2], b: 3}\n");
        assert!(events.contains(&YAMLEvent::StartObject));
        assert!(events.contains(&YAMLEvent::StartArray));
        assert!(events.contains(&YAMLEvent::Number(Cow::Borrowed("1"))));
        assert!(events.contains(&YAMLEvent::Number(Cow::Borrowed("2"))));
        assert!(events.contains(&YAMLEvent::EndArray));
        assert!(events.contains(&YAMLEvent::ObjectKey(Cow::Borrowed("b"))));
        assert!(events.contains(&YAMLEvent::Number(Cow::Borrowed("3"))));
        assert!(events.contains(&YAMLEvent::EndObject));
    }

    #[test]
    fn test_sequence() {
        let events = collect_events("- 1\n- two\n- true\n");
        assert!(events.contains(&YAMLEvent::StartArray));
        assert!(events.contains(&YAMLEvent::Number(Cow::Borrowed("1"))));
        assert!(events.contains(&YAMLEvent::String(Cow::Borrowed("two"))));
        assert!(events.contains(&YAMLEvent::Boolean(true)));
        assert!(events.contains(&YAMLEvent::EndArray));
    }

    #[test]
    fn test_quoted_scalar_not_typed() {
        // "true" in double quotes must be a String, not Boolean
        let events = collect_events("key: \"true\"\n");
        assert!(events.contains(&YAMLEvent::String(Cow::Borrowed("true"))));
        assert!(!events.contains(&YAMLEvent::Boolean(true)));
    }

    #[test]
    fn test_is_yaml_number() {
        assert!(is_yaml_number("42"));
        assert!(is_yaml_number("-3.14"));
        assert!(is_yaml_number("1.5e10"));
        assert!(is_yaml_number("0xFF"));
        assert!(is_yaml_number("0o777"));
        assert!(is_yaml_number(".inf"));
        assert!(is_yaml_number(".nan"));
        assert!(!is_yaml_number("true"));
        assert!(!is_yaml_number("hello"));
        assert!(!is_yaml_number(""));
        assert!(!is_yaml_number("1e"));
    }
}

fn owned_yaml_event(ev: YAMLEvent<'_>) -> YAMLEvent<'static> {
    match ev {
        YAMLEvent::StreamStart => YAMLEvent::StreamStart,
        YAMLEvent::StreamEnd => YAMLEvent::StreamEnd,
        YAMLEvent::DocumentStart => YAMLEvent::DocumentStart,
        YAMLEvent::DocumentEnd => YAMLEvent::DocumentEnd,
        YAMLEvent::StartObject => YAMLEvent::StartObject,
        YAMLEvent::EndObject => YAMLEvent::EndObject,
        YAMLEvent::StartArray => YAMLEvent::StartArray,
        YAMLEvent::EndArray => YAMLEvent::EndArray,
        YAMLEvent::ObjectKey(s) => YAMLEvent::ObjectKey(Cow::Owned(s.into_owned())),
        YAMLEvent::String(s) => YAMLEvent::String(Cow::Owned(s.into_owned())),
        YAMLEvent::Number(s) => YAMLEvent::Number(Cow::Owned(s.into_owned())),
        YAMLEvent::Boolean(b) => YAMLEvent::Boolean(b),
        YAMLEvent::Null => YAMLEvent::Null,
        YAMLEvent::Eof => YAMLEvent::Eof,
    }
}

