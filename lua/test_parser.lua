-- test_parser.lua
-- Tests for the low-level JSON lexer / event generator.

local parser = require("parser")
local E = parser

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
    ok(name, a == b, string.format("expected %q, got %q", tostring(b), tostring(a)))
end

-- ── Helper: collect all events from a string (all at once, is_ending=true) ─

-- Each call to next_event produces at most one event and consumes some bytes.
-- We loop until EOF or error.
local function collect_events(json_str)
    local gen = parser.JSONEventGenerator.new()
    local events = {}
    local cursor = 0
    local len = #json_str

    while true do
        -- Always pass the full remaining slice with is_ending=true.
        local slice = string.sub(json_str, cursor + 1)
        local w = gen:next_event(slice, true)
        cursor = cursor + w.consumed_bytes

        if w.event then
            events[#events + 1] = { event = w.event, error = w.error }
            if w.event.kind == E.EVENT_EOF then break end
        end
        if w.error and not w.event then
            events[#events + 1] = { error = w.error }
            break
        end
        -- Guard: no progress and no event means we're stuck
        if w.consumed_bytes == 0 and not w.event then break end
    end
    return events
end

-- ── Helper: feed JSON as a sequence of chunks, accumulating a scratch buffer
--   exactly like process_chunk does.  This handles mid-token chunk boundaries
--   (BOM check, incomplete numbers, incomplete strings, etc.).
local function events_from_chunks(chunks)
    local gen = parser.JSONEventGenerator.new()
    local events = {}
    local total = #chunks
    local scratch = ""

    for i, chunk in ipairs(chunks) do
        local is_end = (i == total)
        scratch = scratch .. chunk
        local cursor = 0

        while true do
            local slice = string.sub(scratch, cursor + 1)
            local w = gen:next_event(slice, is_end)
            cursor = cursor + w.consumed_bytes

            if w.event then
                events[#events + 1] = w.event
                if w.event.kind == E.EVENT_EOF then
                    scratch = string.sub(scratch, cursor + 1)
                    goto all_done
                end
            end
            if w.error and not w.event then
                scratch = string.sub(scratch, cursor + 1)
                goto all_done
            end
            -- No progress — need more data from next chunk
            if w.consumed_bytes == 0 and not w.event then break end
            -- For non-final chunks, stop when we've consumed everything
            if not is_end and cursor >= #scratch then break end
            -- For the final chunk, keep looping until EOF or no progress
        end
        -- Drain consumed bytes; keep remainder for next chunk
        scratch = string.sub(scratch, cursor + 1)
    end
    ::all_done::
    return events
end

-- ── Tests ───────────────────────────────────────────────────────────────────

print("\n=== test_parser.lua ===\n")

-- Basic null
do
    local evs = collect_events("null")
    eq("null: event count", #evs, 2)
    eq("null: first kind", evs[1].event.kind, E.EVENT_NULL)
    eq("null: second kind", evs[2].event.kind, E.EVENT_EOF)
end

-- Basic boolean true
do
    local evs = collect_events("true")
    eq("true: kind", evs[1].event.kind, E.EVENT_BOOLEAN)
    eq("true: value", evs[1].event.value, true)
end

-- Basic boolean false
do
    local evs = collect_events("false")
    eq("false: kind", evs[1].event.kind, E.EVENT_BOOLEAN)
    eq("false: value", evs[1].event.value, false)
end

-- String
do
    local evs = collect_events('"hello"')
    eq("string: kind", evs[1].event.kind, E.EVENT_STRING)
    eq("string: value", evs[1].event.value, "hello")
end

-- Integer number
do
    local evs = collect_events("42")
    eq("number int: kind", evs[1].event.kind, E.EVENT_NUMBER)
    eq("number int: value", evs[1].event.value, "42")
end

-- Negative number
do
    local evs = collect_events("-7")
    eq("number neg: kind", evs[1].event.kind, E.EVENT_NUMBER)
    eq("number neg: value", evs[1].event.value, "-7")
end

-- Float
do
    local evs = collect_events("3.14")
    eq("number float: kind", evs[1].event.kind, E.EVENT_NUMBER)
    eq("number float: value", evs[1].event.value, "3.14")
end

-- Scientific notation
do
    local evs = collect_events("1.5e10")
    eq("number sci: kind", evs[1].event.kind, E.EVENT_NUMBER)
    eq("number sci: value", evs[1].event.value, "1.5e10")
end

-- Empty object
do
    local evs = collect_events("{}")
    eq("empty object: start kind", evs[1].event.kind, E.EVENT_START_OBJECT)
    eq("empty object: end kind", evs[2].event.kind, E.EVENT_END_OBJECT)
    eq("empty object: eof kind", evs[3].event.kind, E.EVENT_EOF)
end

-- Empty array
do
    local evs = collect_events("[]")
    eq("empty array: start kind", evs[1].event.kind, E.EVENT_START_ARRAY)
    eq("empty array: end kind", evs[2].event.kind, E.EVENT_END_ARRAY)
end

-- Simple object
do
    local evs = collect_events('{"key":"value"}')
    eq("simple obj: start", evs[1].event.kind, E.EVENT_START_OBJECT)
    eq("simple obj: key kind", evs[2].event.kind, E.EVENT_OBJECT_KEY)
    eq("simple obj: key value", evs[2].event.value, "key")
    eq("simple obj: string kind", evs[3].event.kind, E.EVENT_STRING)
    eq("simple obj: string value", evs[3].event.value, "value")
    eq("simple obj: end", evs[4].event.kind, E.EVENT_END_OBJECT)
end

-- Nested object
do
    local evs = collect_events('{"a":{"b":1}}')
    eq("nested obj: start obj 1", evs[1].event.kind, E.EVENT_START_OBJECT)
    eq("nested obj: key a", evs[2].event.kind, E.EVENT_OBJECT_KEY)
    eq("nested obj: start obj 2", evs[3].event.kind, E.EVENT_START_OBJECT)
    eq("nested obj: key b", evs[4].event.kind, E.EVENT_OBJECT_KEY)
    eq("nested obj: number 1", evs[5].event.kind, E.EVENT_NUMBER)
    eq("nested obj: end obj 2", evs[6].event.kind, E.EVENT_END_OBJECT)
    eq("nested obj: end obj 1", evs[7].event.kind, E.EVENT_END_OBJECT)
end

-- Array with values
do
    local evs = collect_events('[1,2,3]')
    eq("array vals: start", evs[1].event.kind, E.EVENT_START_ARRAY)
    eq("array vals: 1", evs[2].event.value, "1")
    eq("array vals: 2", evs[3].event.value, "2")
    eq("array vals: 3", evs[4].event.value, "3")
    eq("array vals: end", evs[5].event.kind, E.EVENT_END_ARRAY)
end

-- String with escapes
do
    local evs = collect_events('"hello\\nworld"')
    eq("escape newline: value", evs[1].event.value, "hello\nworld")
end

do
    local evs = collect_events('"a\\tb"')
    eq("escape tab: value", evs[1].event.value, "a\tb")
end

do
    local evs = collect_events('"quote\\"inside"')
    eq("escape quote: value", evs[1].event.value, 'quote"inside')
end

do
    local evs = collect_events('"back\\\\slash"')
    eq("escape backslash: value", evs[1].event.value, 'back\\slash')
end

-- Unicode escape \uXXXX
do
    local evs = collect_events('"\\u0041"')  -- 'A'
    eq("unicode escape A: value", evs[1].event.value, "A")
end

do
    local evs = collect_events('"\\u00E9"')  -- 'é' = 0xC3 0xA9
    eq("unicode escape é: value", evs[1].event.value, "\xC3\xA9")
end

-- Surrogate pair: U+1F600 = 😀 (F0 9F 98 80)
do
    local evs = collect_events('"\\uD83D\\uDE00"')
    eq("surrogate pair: kind", evs[1].event.kind, E.EVENT_STRING)
    eq("surrogate pair: value", evs[1].event.value, "\xF0\x9F\x98\x80")
end

-- Whitespace handling
do
    local evs = collect_events('  { "k" : "v" }  ')
    eq("whitespace: start", evs[1].event.kind, E.EVENT_START_OBJECT)
    eq("whitespace: key", evs[2].event.value, "k")
    eq("whitespace: val", evs[3].event.value, "v")
end

-- Null value in object
do
    local evs = collect_events('{"x":null}')
    eq("null in obj: null kind", evs[3].event.kind, E.EVENT_NULL)
end

-- Multiple values in object
do
    local evs = collect_events('{"a":1,"b":2}')
    eq("multi obj: key a", evs[2].event.value, "a")
    eq("multi obj: val 1", evs[3].event.value, "1")
    eq("multi obj: key b", evs[4].event.value, "b")
    eq("multi obj: val 2", evs[5].event.value, "2")
end

-- BOM stripping
do
    local evs = collect_events("\xEF\xBB\xBF42")
    eq("BOM: kind", evs[1].event.kind, E.EVENT_NUMBER)
    eq("BOM: value", evs[1].event.value, "42")
end

-- ── Chunked feeding tests ───────────────────────────────────────────────────

-- Split JSON at every boundary and verify we get the same events.
do
    local json = '{"key":"value"}'
    for split = 1, #json - 1 do
        local c1 = string.sub(json, 1, split)
        local c2 = string.sub(json, split + 1)
        local evs = events_from_chunks({ c1, c2 })
        ok(string.format("chunked split@%d: StartObject", split),
            evs[1] and evs[1].kind == E.EVENT_START_OBJECT, "no StartObject")
        ok(string.format("chunked split@%d: ObjectKey=key", split),
            evs[2] and evs[2].kind == E.EVENT_OBJECT_KEY and evs[2].value == "key",
            string.format("got kind=%s val=%s", tostring(evs[2] and evs[2].kind), tostring(evs[2] and evs[2].value)))
        ok(string.format("chunked split@%d: String=value", split),
            evs[3] and evs[3].kind == E.EVENT_STRING and evs[3].value == "value",
            string.format("got kind=%s val=%s", tostring(evs[3] and evs[3].kind), tostring(evs[3] and evs[3].value)))
        ok(string.format("chunked split@%d: EndObject", split),
            evs[4] and evs[4].kind == E.EVENT_END_OBJECT, "no EndObject")
    end
end

-- Chunk-feed a complex JSON with many split points and compare to batch result.
do
    local json = '{"a":1,"b":[2,3],"c":{"d":true}}'
    local batch = collect_events(json)
    -- Try several specific split positions
    local splits = { 1, 5, 10, 15, 20, #json - 1 }
    for _, sp in ipairs(splits) do
        if sp > 0 and sp < #json then
            local evs = events_from_chunks({
                string.sub(json, 1, sp),
                string.sub(json, sp + 1),
            })
            ok(string.format("complex chunked@%d: count", sp),
                #evs == #batch,
                string.format("batch=%d chunked=%d", #batch, #evs))
        end
    end
end

-- Chunked number split in the middle of digits
do
    local evs = events_from_chunks({ '{"ts":', '12345', '6789}' })
    ok("chunked number: ObjectKey", evs[2] and evs[2].value == "ts", "wrong key")
    ok("chunked number: value", evs[3] and evs[3].value == "123456789", "wrong number")
end

-- Chunked string split inside the string content
do
    local evs = events_from_chunks({ '{"k":"hel', 'lo"}' })
    ok("chunked string: value", evs[3] and evs[3].value == "hello", "wrong string")
end

-- Chunked string split inside an escape sequence
do
    local evs = events_from_chunks({ '{"k":"a\\', 'nb"}' })
    ok("chunked escape: value", evs[3] and evs[3].value == "a\nb",
        string.format("got %q", evs[3] and evs[3].value))
end

-- ── Error detection ─────────────────────────────────────────────────────────

local function has_error(json_str)
    local evs = collect_events(json_str)
    for _, e in ipairs(evs) do
        if e.error then return true end
    end
    return false
end

ok("trailing comma obj: error", has_error('{"a":1,}'), "expected error")
ok("trailing comma arr: error", has_error('[1,2,]'), "expected error")
ok("missing colon: error", has_error('{"a" 1}'), "expected error")
ok("invalid keyword: error", has_error('trueX'), "expected error")
ok("bad number start: error", has_error('a42'), "expected error")
ok("invalid escape: error", has_error('"\\q"'), "expected error")

-- ── Large JSON end-to-end ───────────────────────────────────────────────────

do
    local json = '{"name":"Alice","scores":[10,20,30],"meta":{"active":true,"count":null}}'
    local evs = collect_events(json)
    -- Just verify we get StartObject and eventually EndObject without errors
    local had_error = false
    local kinds = {}
    for _, e in ipairs(evs) do
        if e.error then had_error = true end
        if e.event then kinds[#kinds + 1] = e.event.kind end
    end
    ok("large json: no errors", not had_error, "unexpected error")
    eq("large json: first event", kinds[1], E.EVENT_START_OBJECT)
    eq("large json: last event before EOF", kinds[#kinds - 1], E.EVENT_END_OBJECT)
end

-- ── Summary ─────────────────────────────────────────────────────────────────

print(string.format("\n%d passed, %d failed\n", pass_count, fail_count))
if fail_count > 0 then os.exit(1) end
