use std::collections::{HashMap};
use serde_json::{Value};
use json_chunk::parser::{JSONEvent, JSONEventGenerator, JSONEventWrapper};

#[allow(dead_code)]
pub fn print_jsons_with_chunk_info(expected: &HashMap<String, (usize,Value)>, matches_found: &HashMap<String, Value>, result: &Value, verbose: bool) {
    println!("expected ({}):", expected.len());
    print_json_chunkinfo(&expected, verbose);
    println!("parser.matches_found ({}):", matches_found.len());
    print_mapjson_summary(matches_found, verbose);
    println!("Result JSON:");
    print_json_summary(result, verbose);
}

#[allow(dead_code)]
pub fn print_jsons(expected: &HashMap<String, Value>, matches_found: &HashMap<String, Value>, result: &Value, verbose: bool) {
    println!("expected ({}):", expected.len());
    print_mapjson_summary(&expected, verbose);
    println!("parser.matches_found ({}):", matches_found.len());
    print_mapjson_summary(matches_found, verbose);
    println!("Result JSON:");
    print_json_summary(result, verbose);
}

/// Minimal LCG so we don't need the `rand` crate.
/// Walk `bytes` as JSON and print its structure to stdout.
/// String values are summarised as:
///   `string of N chars starting with "XYZ" and ending with "XYZ"`
/// so that very long values don't flood the terminal.
#[allow(dead_code)]
pub fn print_json_structure(bytes: &[u8]) {
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
            parser.next_event(&bytes[cursor..], true);
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

#[allow(dead_code)]
pub fn find_and_print_key_in_chunk(key: &str, chunk1: &Vec<u8>, next_chunks: &[&Vec<u8>], chunk_idx: usize, check_overlap: bool, prefix: &str, search_from: usize) -> Option<usize> {
    if key == "" {
        return None;
    }
    let mut field = "field";
    if prefix != "" {
        field = prefix;
    }
    let content = String::from_utf8_lossy(chunk1);
    let search_slice = if search_from < content.len() {
        &content[search_from..]
    } else {
        return None;
    };
    if let Some(relative_pos) = search_slice.find(key) {
        let s: usize = search_from + relative_pos;
        let start: usize = s.saturating_sub(10);
        let end: usize = (s + key.len()+30).min(content.len());
        // fixed width columns: | prefix | field | chunk_idx | chunk_len | snippet
        println!("| {:<15} | {:<20} | {:>15} | {:>15} | chunk#{}: ```{}```",
            field, key, format!("{}[{}]", chunk_idx, start), "", chunk_idx, &content[start..end]);
        return Some(s + key.len());
    }
    if check_overlap {
        let prev_chunk = &search_slice.as_bytes().to_vec();
        let mut joined = Vec::with_capacity(chunk1.len()*2);
        joined.extend_from_slice(&prev_chunk);
        for c in next_chunks {
            joined.extend_from_slice(&c);
        }
        let search_content = String::from_utf8_lossy(&joined);
        if let Some(relative_pos) = search_content.find(key) {
            let s: usize = search_from + relative_pos;
            let start: usize = s.saturating_sub(10);
            // across chunks: show both parts in fixed columns
            print!("| {:<15} | {:<20} | {:>15} | {:>15} | chunk#{}: ```{}```",
                field, key, format!("{}[{}]", chunk_idx, start), chunk_idx+1, chunk_idx, &content);
            for (i, c ) in next_chunks.iter().enumerate() {
                print!("  >>>>  chunk#{}: ```{}```", chunk_idx+i+1, String::from_utf8_lossy(&c));
            }
            println!();
            return Some(s + key.len());
        }
    }
    None
}

#[allow(dead_code)]
pub fn find_and_print_in_single_or_overlap(key: &str, chunks: &Vec<Vec<u8>>, chunk_idx: usize, total: usize, prefix: &str, depth: usize) -> bool {
    let chunk = &chunks[chunk_idx];
    let mut next_chunks: Vec<&Vec<u8>> = Vec::new();
    for i in 1..depth {
        if chunk_idx + i < total {
            next_chunks.push(&chunks[chunk_idx+i]);
        }        
    }
    let mut search_from = 0;
    let mut found_any = false;
    loop {
        if let Some(next_pos) = find_and_print_key_in_chunk(key, &chunk, &next_chunks, chunk_idx, chunk_idx + 1 < total, prefix, search_from) {
            found_any = true;
            search_from = next_pos;
        } else {
            break;
        }
    }
    found_any
}

#[allow(dead_code)]
pub fn print_relevant_chunks(all_names: &Vec<&str>, successors: &HashMap<&str, &str>, chunks: &Vec<Vec<u8>>, depth: usize) {
    let total: usize = chunks.len();
    // for (i, c) in chunks.iter().enumerate() {
    //     println!("chunk# {}:", i);
    //     let s = String::from_utf8_lossy(&c);
    //     println!("{}", s)
    // }
    println!("\nField distribution across {} chunks for target fields {:?}:", total, all_names);
    let mut matched_successors: Vec<&&str> = Vec::new();
    println!("| {:<15} | {:<20} | {:>15} | {:>15} | {}",
            "kind", "field name", "from chunk", "to chunk", "chunk");
    for (i, _) in chunks.iter().enumerate() {
      for name in all_names.iter() {
        if find_and_print_in_single_or_overlap(name, chunks, i, total, "", depth) {
          matched_successors.push(successors.get(name).unwrap_or(&""));
        }
      }
      for name in matched_successors.clone().iter() {
        if find_and_print_in_single_or_overlap(name, chunks, i, total, "successor", depth) {
          matched_successors.remove(0);
        }
      }
    }
    println!();
}

#[allow(dead_code)]
pub fn print_kv(k: &String, v: &Value, verbose: bool) {
    let value = serde_json::to_string(&v).unwrap();
    let len = value.len()-2;
    if verbose || len < 10 {
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

#[allow(dead_code)]
pub fn print_json_chunkinfo(json : &HashMap<String, (usize,Value)>, verbose: bool) {
    let empty: String = "\"\"".to_string();
    println!("{{");
    for (mut k, v) in json.iter() {
        if k == "" {
            k = &empty;
        }
        println!("In chunk# {}", &v.0);
        print_kv(k, &v.1, verbose);
    }
    println!("}}");
}

#[allow(dead_code)]
pub fn print_mapjson_summary(json : &HashMap<String, Value>, verbose: bool) {
    let empty: String = "\"\"".to_string();
    println!("{{");
    for (mut k, v) in json.iter() {
        if k == "" {
            k = &empty;
        }
        print_kv(k, v, verbose);
    }
    println!("}}");
}

#[allow(dead_code)]
pub fn print_json_summary(json : &Value, verbose: bool) {
    println!("{{");
    if let Value::Object(map) = json {
        for (key, value) in map {
            print_kv(key, &value, verbose);
        }
    } else {
        println!("not an object: {}", json);
    }
    println!("}}");
}
