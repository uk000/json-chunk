-- test_chunk_parser.lua
-- Tests for the streaming JSON chunk parser.
-- Mirrors rust/tests/chunk_parser_tests.rs with value-level assertions.

local cp_module = require("chunk_parser")
local helpers   = require("test_helpers")

local ChunkParser = cp_module.ChunkParser

-- ── Minimal test framework ──────────────────────────────────────────────────

local pass_count = 0
local fail_count = 0

local function ok(name, cond, msg)
    if cond then
        pass_count = pass_count + 1
        io.write(string.format("  PASS  %s\n", name))
    else
        fail_count = fail_count + 1
        io.write(string.format("  FAIL  %s: %s\n", name, msg or "assertion failed"))
    end
end

local function eq(name, a, b)
    local same
    if type(a) == "number" and type(b) == "number" then
        same = (a == b)
    elseif type(a) == "number" and type(b) == "string" then
        same = (a == tonumber(b))
    elseif type(a) == "string" and type(b) == "number" then
        same = (tonumber(a) == b)
    else
        same = (a == b)
    end
    ok(name, same, string.format("expected %q, got %q", tostring(b), tostring(a)))
end

-- Deep-equal two values (tables compared recursively).
local function deep_eq(a, b)
    if a == b then return true end
    if type(a) ~= type(b) then
        -- number/string coercion for JSON numbers
        if type(a) == "number" and type(b) == "string" then return a == tonumber(b) end
        if type(a) == "string" and type(b) == "number" then return tonumber(a) == b end
        return false
    end
    if type(a) ~= "table" then return false end
    if a == cp_module.NULL then return b == cp_module.NULL end
    for k, v in pairs(a) do
        if k ~= "is_array" then
            if not deep_eq(v, b[k]) then return false end
        end
    end
    for k in pairs(b) do
        if k ~= "is_array" and a[k] == nil then return false end
    end
    return true
end

local function ok_eq(name, a, b)
    ok(name, deep_eq(a, b),
        string.format("expected %s, got %s", tostring(b), tostring(a)))
end

-- ── test_happy_paths helper (mirrors Rust's test_happy_paths) ───────────────

local function test_happy_paths(name, json_str, path_map)
    local cp = ChunkParser.new_json_parser(path_map)
    local keys = helpers.get_expected_field_names(path_map)
    local chunks = helpers.random_chunks(json_str, 10, 50, 42, true, keys)
    local expected_with_pos = helpers.build_expected_with_pos(path_map, chunks)

    local last_chunk = helpers.feed_chunks_to_parser(cp, chunks)

    ok(name .. ": is_all_found", cp:is_all_found(), "expected found")
    ok(name .. ": last_chunk not nil", last_chunk ~= nil, "expected last_chunk")

    if last_chunk == nil then return end

    -- Verify each matched value
    local matches = cp:get_matches()
    for path, opts in pairs(path_map) do
        local out_key = opts[1] or path
        local exp_entry = expected_with_pos[out_key]
        if exp_entry then
            ok(name .. ": " .. out_key .. " found", matches[out_key] ~= nil,
                "value missing from matches")
        end
    end

    -- Verify the parser exited at exactly the right chunk
    -- (the chunk where the last path first became readable)
    local expected_last = 0
    for path, opts in pairs(path_map) do
        local out_key = opts[1] or path
        local exp_entry = expected_with_pos[out_key]
        if exp_entry and exp_entry[1] > expected_last then
            expected_last = exp_entry[1]
        end
    end
    eq(name .. ": last chunk index", last_chunk, expected_last)
end

-- ── Tests ───────────────────────────────────────────────────────────────────

print("\n=== test_chunk_parser.lua ===\n")

-- ── test_detect_json_end_with_no_end_of_stream ──────────────────────────────

do
    print("-- test_detect_json_end_with_no_end_of_stream")
    local all_expected = {}
    local json_str = helpers.build_invalid_json(3, 10, all_expected)

    local path_map = {
        ["config.key"]        = { nil, 100 },
        ["config.values.xxx"] = { "value", 0 },
    }

    local keys = helpers.get_expected_field_names(path_map)
    local chunks = helpers.random_chunks(json_str, 50, 300, 42, true, keys)

    local cp = ChunkParser.new_json_parser(path_map)
    for _, chunk in ipairs(chunks) do
        cp:process_chunk(chunk, false)  -- never signal end-of-stream
        if cp:is_all_found() then break end
    end

    ok("detect_json_end: is_all_found == false", not cp:is_all_found(),
        "expected is_all_found() == false (no end-of-stream signal)")
end

-- ── test_short_circuit_early_finish ─────────────────────────────────────────

do
    print("-- test_short_circuit_early_finish")
    local all_expected = {}
    local json_str = helpers.build_large_json(10, 10, all_expected)

    test_happy_paths("short_circuit 1field", json_str, {
        ["field_0"] = { nil, 100 },
    })

    test_happy_paths("short_circuit 2fields", json_str, {
        ["field_0"] = { nil, 100 },
        ["field_9"] = { nil, 100 },
    })

    test_happy_paths("short_circuit 3fields", json_str, {
        ["field_0"]         = { nil, 100 },
        ["field_9"]         = { nil, 100 },
        ["metadata.author"] = { nil, 100 },
    })
end

-- ── test_object_with_empty_fields ────────────────────────────────────────────

do
    print("-- test_object_with_empty_fields")
    local FIELD_COUNT = 3
    local VALUE_LEN   = 10
    local all_expected = {}
    local json_str = helpers.build_small_json(FIELD_COUNT, VALUE_LEN, all_expected)

    -- Ground-truth values derived by the same formulas as build_small_json
    local locale_val = helpers.text_value('F', VALUE_LEN, ' ')
    local key_val    = helpers.text_value('K', VALUE_LEN, ' ')
    local value2_val = helpers.text_value('M', VALUE_LEN, ' ')
    local author_val = helpers.text_value('A', VALUE_LEN, ' ')

    local path_map = {
        ["metadata."]                     = { "metadata", 100 },
        ["metadata.stats.details.locale"] = { "locale",   100 },
        ["config.key"]                    = { nil,        100 },
        ["config.values.value2"]          = { "value",    0   },
        ["."]                             = { "default",  100 },
    }

    local keys = helpers.get_expected_field_names(path_map)
    local chunks = helpers.random_chunks(json_str, 10, 50, 42, true, keys)
    local total = #chunks

    local cp = ChunkParser.new_json_parser(path_map)
    for i, chunk in ipairs(chunks) do
        cp:process_chunk(chunk, i == total)
        if cp:is_all_found() then break end
    end

    ok("empty_fields: is_all_found", cp:is_all_found(), "expected is_all_found")
    ok("empty_fields: locale not overflow",
        not cp:get_field("metadata.stats.details.locale").overflow, "overflow==false")
    ok("empty_fields: config.key not overflow",
        not cp:get_field("config.key").overflow, "overflow==false")
    ok("empty_fields: value2 not overflow",
        not cp:get_field("config.values.value2").overflow, "overflow==false")

    local m = cp:get_matches()

    -- Value-level assertions (mirrors Rust's assert_eq!(json, to_value(expected)))
    eq("empty_fields: locale == expected",
        m["locale"], locale_val)
    eq("empty_fields: config.key == expected",
        m["config.key"], key_val)
    eq("empty_fields: value == expected",
        m["value"], value2_val)
    -- metadata is the value of the "" key inside "metadata" object (= author)
    eq("empty_fields: metadata == expected",
        m["metadata"], author_val)
    -- "default" is the root-level "" key (= tags array) — check it's a table
    ok("empty_fields: default is table",
        type(m["default"]) == "table", "expected table for tags array")
    ok("empty_fields: default is array",
        m["default"] and m["default"].is_array, "expected is_array")
end

-- ── test_fields_overflow ─────────────────────────────────────────────────────

do
    print("-- test_fields_overflow")
    local all_expected = {}
    local json_str = helpers.build_large_json(3, 1000, all_expected)

    local field1_val = helpers.text_value('a', 1000, '"')  -- field_0 is 'a', field_1 is 'b'
    -- field_1 uses char 'b' (index 1, 0-based → 'a'+1='b')
    field1_val = helpers.rep('b', 1000)
    local value2_val = helpers.text_value('M', 1000, ' ')

    local path_map = {
        ["field_1"]                         = { "b",      0   },
        ["metadata.stats.details.locale"]   = { "locale", 200 },
        ["config.key"]                      = { nil,      100 },
        ["config.values.value2"]            = { "value",  0   },
    }

    local keys = helpers.get_expected_field_names(path_map)
    local chunks = helpers.random_chunks(json_str, 50, 300, 42, true, keys)

    local cp = ChunkParser.new_json_parser(path_map)
    helpers.feed_chunks_to_parser(cp, chunks)

    ok("overflow: is_all_found", cp:is_all_found(), "expected is_all_found")
    ok("overflow: field_1 not overflow", not cp:get_field("field_1").overflow,
        "expected field_1 overflow == false")
    ok("overflow: locale overflow", cp:get_field("metadata.stats.details.locale").overflow,
        "expected locale overflow == true")
    ok("overflow: config.key overflow", cp:get_field("config.key").overflow,
        "expected config.key overflow == true")
    ok("overflow: value2 not overflow", not cp:get_field("config.values.value2").overflow,
        "expected value2 overflow == false")

    local m = cp:get_matches()

    -- Non-overflowed fields must be present with correct values
    eq("overflow: field_1 value",    m["b"],     field1_val)
    eq("overflow: value2 value",     m["value"], value2_val)

    -- Overflowed fields must NOT appear in matches
    ok("overflow: locale not in matches",   m["locale"] == nil, "overflowed field in matches")
    ok("overflow: config.key not in matches", m["config.key"] == nil, "overflowed field in matches")
end

-- ── test_split_numeric_field ─────────────────────────────────────────────────

do
    print("-- test_split_numeric_field")
    local json_str = string.format('{"timestamp":%d}', 123456789)

    local path_map = { ["timestamp"] = { nil, 100 } }
    local keys = helpers.get_expected_field_names(path_map)
    local chunks = helpers.random_chunks(json_str, 10, 50, 42, true, keys)

    local cp = ChunkParser.new_json_parser(path_map)
    helpers.feed_chunks_to_parser(cp, chunks)

    ok("split_numeric: is_all_found", cp:is_all_found(), "expected found")
    local m = cp:get_matches()
    eq("split_numeric: timestamp value", m["timestamp"], 123456789)
end

-- ── test_mix_flat_nested_fields_in_random_chunks ─────────────────────────────

do
    print("-- test_mix_flat_nested_fields_in_random_chunks")
    local all_expected = {}
    local json_str = helpers.build_large_json(10, 50, all_expected)

    local locale_val = helpers.text_value('F', 50, ' ')
    local key_val    = helpers.text_value('K', 50, ' ')
    local value2_val = helpers.text_value('M', 50, ' ')
    local field1_val = helpers.rep('b', 50)

    local path_map = {
        ["field_1"]                         = { "b",      0   },
        ["metadata.stats.details.locale"]   = { "locale", 100 },
        ["config.key"]                      = { nil,      100 },
        ["config.values.value2"]            = { "value",  0   },
        ["timestamp"]                       = { nil,      10  },
    }
    -- Note: "timestamp" has max_size=10 and value "123456789" (9 chars) → fits
    -- locale is 52 chars (space+50F+space) which is > 100? No, 52 < 100 so no overflow

    local keys = helpers.get_expected_field_names(path_map)
    local chunks = helpers.random_chunks(json_str, 10, 50, 42, true, keys)

    local cp = ChunkParser.new_json_parser(path_map)
    local last = helpers.feed_chunks_to_parser(cp, chunks)

    ok("mix_flat_nested: is_all_found", cp:is_all_found(), "expected found")
    ok("mix_flat_nested: last_chunk not nil", last ~= nil, "expected last_chunk")

    local m = cp:get_matches()
    eq("mix_flat_nested: locale",    m["locale"],     locale_val)
    eq("mix_flat_nested: config.key", m["config.key"], key_val)
    eq("mix_flat_nested: value2",    m["value"],      value2_val)
    eq("mix_flat_nested: field_1",   m["b"],          field1_val)
    eq("mix_flat_nested: timestamp", m["timestamp"],  123456789)
end

-- ── test_multi_nested_obj_arrays_in_random_chunks ────────────────────────────

do
    print("-- test_multi_nested_obj_arrays_in_random_chunks")
    local all_expected = {}
    local json_str = helpers.build_large_json(2, 10, all_expected)

    local name_val = helpers.text_value('G', 10, ' ')

    local path_map = {
        ["metadata.stats.details.regions"] = { "regions", 0 },
        ["config.values"]                  = { "values",  0 },
        ["metadata.name"]                  = { nil,       0 },
    }

    test_happy_paths("multi_nested", json_str, path_map)

    -- Also check structural correctness for container values
    local keys = helpers.get_expected_field_names(path_map)
    local chunks = helpers.random_chunks(json_str, 10, 50, 42, true, keys)
    local cp = ChunkParser.new_json_parser(path_map)
    helpers.feed_chunks_to_parser(cp, chunks)

    local m = cp:get_matches()
    ok("multi_nested: regions is array",
        m["regions"] and m["regions"].is_array, "expected array")
    ok("multi_nested: values is object",
        m["values"] and not m["values"].is_array, "expected object")
    eq("multi_nested: metadata.name", m["metadata.name"], name_val)
end

-- ── test_invalid_json_fields ──────────────────────────────────────────────────

do
    print("-- test_invalid_json_fields")
    local all_expected = {}
    local json_str = helpers.build_large_json(20, 2000, all_expected)

    local path_map = {
        ["field_x"]                       = { nil,      0   },
        ["metadata.foo.details.region"]   = { "region", 512 },
        ["foo.name"]                      = { nil,      256 },
    }

    local chunks = helpers.random_chunks(json_str, 50, 300, 42, false, {})
    local total = #chunks

    local cp = ChunkParser.new_json_parser(path_map)
    for i, chunk in ipairs(chunks) do
        cp:process_chunk(chunk, i == total)
        if cp:is_all_found() then break end
    end

    ok("invalid_fields: is_all_found == false", not cp:is_all_found(),
        "expected not found (paths don't exist)")

    -- No matches should be found (Rust: assert_eq!(json, Value::Object({})))
    local m = cp:get_matches()
    local match_count = 0
    for _ in pairs(m) do match_count = match_count + 1 end
    eq("invalid_fields: zero matches", match_count, 0)
end

-- ── test_mix_valid_and_invalid_fields ────────────────────────────────────────

do
    print("-- test_mix_valid_and_invalid_fields")
    local all_expected = {}
    local json_str = helpers.build_large_json(10, 1000, all_expected)

    local path_map = {
        ["field_x"]                         = { "b",      1024 },
        ["metadata.stats.details.locale"]   = { "locale", 100  },
        ["config.stamp"]                    = { nil,      0    },
        ["timestamp"]                       = { nil,      0    },
    }

    local keys = helpers.get_expected_field_names(path_map)
    local chunks = helpers.random_chunks(json_str, 50, 300, 42, true, keys)
    local total = #chunks

    local cp = ChunkParser.new_json_parser(path_map)
    for i, chunk in ipairs(chunks) do
        cp:process_chunk(chunk, i == total)
        if cp:is_all_found() then break end
    end

    -- field_x and config.stamp don't exist; locale overflows; timestamp found
    ok("mix_valid_invalid: is_all_found == false", not cp:is_all_found(),
        "expected not all found (field_x and config.stamp missing)")
    ok("mix_valid_invalid: locale overflow",
        cp:get_field("metadata.stats.details.locale").overflow,
        "expected locale overflow == true")

    -- Rust: assert_ne!(json, serde_json::to_value(expected).unwrap())
    -- expected would include "timestamp"; the result should differ because
    -- field_x and config.stamp are missing.
    local m = cp:get_matches()
    ok("mix_valid_invalid: timestamp found", m["timestamp"] == 123456789,
        "timestamp should be found")
    ok("mix_valid_invalid: field_x not found", m["b"] == nil,
        "field_x doesn't exist, should be absent")
    ok("mix_valid_invalid: locale not in matches", m["locale"] == nil,
        "overflowed locale should not be in matches")
end

-- ── test_flat_text_fields ────────────────────────────────────────────────────

do
    print("-- test_flat_text_fields")
    local text_str = helpers.build_text_input(30, 10)

    local path_map = {
        ["field_0"] = { nil, 0 },
        ["field_2"] = { nil, 0 },
        ["field_5"] = { nil, 0 },
    }

    local keys = helpers.get_expected_field_names(path_map)
    local chunks = helpers.random_chunks(text_str, 50, 300, 42, true, keys)
    local total = #chunks

    local cp = ChunkParser.new_json_parser(path_map)
    for i, chunk in ipairs(chunks) do
        cp:process_chunk(chunk, i == total)
        if cp:is_all_found() then break end
    end

    -- Plain text is not JSON; parser won't find the fields
    -- (Rust test: no assertion on is_all_found, just runs without crash)
    ok("flat_text: ran without error", true, "")
    -- The matches should be empty since it's not valid JSON
    local m = cp:get_matches()
    local count = 0
    for _ in pairs(m) do count = count + 1 end
    eq("flat_text: no matches found", count, 0)
end

-- ── Additional tests: single-chunk, output-key remapping, etc. ───────────────

do
    print("-- test_single_chunk")
    local json_str = '{"name":"Alice","age":30,"active":true}'
    local path_map = {
        ["name"]   = { nil, 100 },
        ["age"]    = { nil, 100 },
        ["active"] = { nil, 100 },
    }

    local cp = ChunkParser.new_json_parser(path_map)
    cp:process_chunk(json_str, true)

    ok("single_chunk: is_all_found", cp:is_all_found(), "expected found")
    local m = cp:get_matches()
    eq("single_chunk: name",   m["name"],   "Alice")
    eq("single_chunk: age",    m["age"],    30)
    eq("single_chunk: active", m["active"], true)
end

do
    print("-- test_output_key_remapping")
    local json_str = '{"config":{"host":"localhost","port":8080}}'

    local path_map = {
        ["config.host"] = { "server_host", 100 },
        ["config.port"] = { "server_port", 100 },
    }

    local cp = ChunkParser.new_json_parser(path_map)
    cp:process_chunk(json_str, true)

    ok("output_key: is_all_found", cp:is_all_found(), "expected found")
    local m = cp:get_matches()
    eq("output_key: server_host", m["server_host"], "localhost")
    eq("output_key: server_port", m["server_port"], 8080)
    ok("output_key: config.host absent", m["config.host"] == nil, "should be remapped")
    ok("output_key: config.port absent", m["config.port"] == nil, "should be remapped")
end

do
    print("-- test_nested_array_extraction")
    local json_str = '{"data":{"items":[1,2,3]}}'
    local path_map = { ["data.items"] = { nil, 0 } }

    local cp = ChunkParser.new_json_parser(path_map)
    cp:process_chunk(json_str, true)

    ok("nested_array: is_all_found", cp:is_all_found(), "expected found")
    local m = cp:get_matches()
    ok("nested_array: items is array", m["data.items"] and m["data.items"].is_array, "expected array")
    eq("nested_array: items[1]", m["data.items"][1], 1)
    eq("nested_array: items[2]", m["data.items"][2], 2)
    eq("nested_array: items[3]", m["data.items"][3], 3)
end

do
    print("-- test_get_result_json")
    local json_str = '{"x":42,"y":"hello"}'
    local path_map = {
        ["x"] = { nil, 100 },
        ["y"] = { nil, 100 },
    }

    local cp = ChunkParser.new_json_parser(path_map)
    cp:process_chunk(json_str, true)

    ok("result_json: is_all_found", cp:is_all_found(), "expected found")
    local result_json = cp:get_result_json()
    -- Must be valid JSON with both fields
    ok("result_json: contains x", result_json:find('"x"') ~= nil, "missing x")
    ok("result_json: contains y", result_json:find('"y"') ~= nil, "missing y")
    ok("result_json: contains 42", result_json:find('42') ~= nil, "missing 42")
    ok("result_json: contains hello", result_json:find('"hello"') ~= nil, "missing hello")
end

do
    print("-- test_size_overflow")
    local long_value = string.rep("x", 10000)
    local json_str = string.format('{"big":"%s"}', long_value)
    local path_map = { ["big"] = { nil, 50 } }

    local cp = ChunkParser.new_json_parser(path_map)
    cp:process_chunk(json_str, true)

    ok("size_overflow: overflow flag", cp:get_field("big").overflow, "expected overflow")
    ok("size_overflow: is_all_found (overflow counts)", cp:is_all_found(), "expected found")
    local m = cp:get_matches()
    ok("size_overflow: big absent from matches", m["big"] == nil, "overflowed should be absent")
end

do
    print("-- test_process_chunks_method")
    local json_str = '{"x":42,"y":"hello"}'
    local path_map = {
        ["x"] = { nil, 100 },
        ["y"] = { nil, 100 },
    }

    local cp = ChunkParser.new_json_parser(path_map)
    local chunks = {}
    for i = 1, #json_str, 3 do
        chunks[#chunks + 1] = string.sub(json_str, i, i + 2)
    end
    cp:process_chunks(chunks)

    ok("process_chunks: is_all_found", cp:is_all_found(), "expected found")
    local m = cp:get_matches()
    eq("process_chunks: x", m["x"], 42)
    eq("process_chunks: y", m["y"], "hello")
end

-- ── Summary ──────────────────────────────────────────────────────────────────

print(string.format("\n%d passed, %d failed\n", pass_count, fail_count))
if fail_count > 0 then os.exit(1) end
