-- chunk_parser.lua
-- Streaming JSON path extractor that processes JSON given in chunks.
-- Ported from rust/src/chunk_parser.rs.
--
-- No external JSON library is used. A minimal JSON value decoder is built
-- on top of the JSONEventGenerator from parser.lua.

local parser = require("parser")

local M = {}

-- ── Minimal JSON value decoder (no cjson) ─────────────────────────────────
-- Produces Lua-native values:
--   JSON null   → the singleton M.NULL (a unique table)
--   JSON bool   → Lua boolean
--   JSON number → Lua number
--   JSON string → Lua string
--   JSON array  → Lua array-table  { is_array=true, [1..n]=... }
--   JSON object → Lua hash-table   { is_array=false, key=value, ... }

M.NULL = setmetatable({}, { __tostring = function() return "null" end })

-- Parse a single JSON value from the string `buf`.
-- Returns the value, or nil on failure.
local function decode_json_value(buf)
    local gen = parser.JSONEventGenerator.new()
    local cursor = 0
    local len = #buf

    -- Iterative stack-based parser
    local value_stack = {}   -- each entry: { kind="array"|"object", data=table, key=nil }
    local result = nil

    local function push_value(v)
        if #value_stack == 0 then
            result = v
        else
            local top = value_stack[#value_stack]
            if top.kind == "array" then
                top.data[#top.data + 1] = v
            else
                top.pending_value = v
                if top.key then
                    top.data[top.key] = v
                    top.key = nil
                end
            end
        end
    end

    while cursor <= len do
        local slice = string.sub(buf, cursor + 1)
        local w = gen:next_event(slice, true)
        cursor = cursor + w.consumed_bytes

        if w.error and not w.event then
            return nil
        end

        local ev = w.event
        if ev == nil then break end

        local ek = ev.kind

        if ek == parser.EVENT_EOF then
            break
        elseif ek == parser.EVENT_NULL then
            push_value(M.NULL)
        elseif ek == parser.EVENT_BOOLEAN then
            push_value(ev.value)
        elseif ek == parser.EVENT_NUMBER then
            push_value(tonumber(ev.value))
        elseif ek == parser.EVENT_STRING then
            push_value(ev.value)
        elseif ek == parser.EVENT_START_OBJECT then
            local obj = { is_array = false }
            value_stack[#value_stack + 1] = { kind = "object", data = obj, key = nil }
        elseif ek == parser.EVENT_END_OBJECT then
            local top = table.remove(value_stack)
            push_value(top.data)
        elseif ek == parser.EVENT_START_ARRAY then
            local arr = { is_array = true }
            value_stack[#value_stack + 1] = { kind = "array", data = arr }
        elseif ek == parser.EVENT_END_ARRAY then
            local top = table.remove(value_stack)
            push_value(top.data)
        elseif ek == parser.EVENT_OBJECT_KEY then
            local top = value_stack[#value_stack]
            top.key = ev.value
        end
    end

    return result
end

-- Encode a Lua value (produced by decode_json_value) back to a JSON string.
-- Used for get_result_json().
local function encode_json_value(v)
    if v == nil then
        return "null"
    elseif v == M.NULL then
        return "null"
    elseif type(v) == "boolean" then
        return v and "true" or "false"
    elseif type(v) == "number" then
        -- Preserve integer representation when possible
        if v == math.floor(v) and math.abs(v) < 1e15 then
            return string.format("%d", v)
        else
            return string.format("%.17g", v)
        end
    elseif type(v) == "string" then
        -- JSON-encode the string
        local out = { '"' }
        for i = 1, #v do
            local b = string.byte(v, i)
            if b == 0x22 then out[#out+1] = '\\"'
            elseif b == 0x5C then out[#out+1] = '\\\\'
            elseif b == 0x08 then out[#out+1] = '\\b'
            elseif b == 0x0C then out[#out+1] = '\\f'
            elseif b == 0x0A then out[#out+1] = '\\n'
            elseif b == 0x0D then out[#out+1] = '\\r'
            elseif b == 0x09 then out[#out+1] = '\\t'
            elseif b < 0x20 then
                out[#out+1] = string.format('\\u%04X', b)
            else
                out[#out+1] = string.char(b)
            end
        end
        out[#out+1] = '"'
        return table.concat(out)
    elseif type(v) == "table" then
        if v.is_array then
            local parts = {}
            for i = 1, #v do
                parts[i] = encode_json_value(v[i])
            end
            return "[" .. table.concat(parts, ",") .. "]"
        else
            local parts = {}
            -- Sort keys for determinism
            local keys = {}
            for k in pairs(v) do
                if k ~= "is_array" then keys[#keys+1] = k end
            end
            table.sort(keys)
            for _, k in ipairs(keys) do
                parts[#parts+1] = encode_json_value(k) .. ":" .. encode_json_value(v[k])
            end
            return "{" .. table.concat(parts, ",") .. "}"
        end
    end
    return "null"
end

M.decode_json_value = decode_json_value
M.encode_json_value = encode_json_value

-- ── PathTracker ────────────────────────────────────────────────────────────

local PathTracker = {}
PathTracker.__index = PathTracker

function PathTracker.new(json_path, output_key, max_size)
    local parts = {}
    for part in (json_path .. "."):gmatch("([^.]*)%.") do
        parts[#parts + 1] = part
    end
    return setmetatable({
        path             = json_path,
        path_vector      = parts,
        output_key       = output_key,   -- string or nil
        max_value_length = max_size,     -- 0 = unlimited
        matched_depth    = 0,
        array_nesting    = 0,
        skipped_depth    = 0,
        current_key      = nil,
        done             = false,
        collecting_depth = 0,
        collect_buffer   = "",
        overflow         = false,
    }, PathTracker)
end

function PathTracker:is_collecting()
    return self.collecting_depth > 0
end

function PathTracker:is_skipping()
    return self.skipped_depth > 0
end

function PathTracker:set_current_key(key)
    self.current_key = key
end

function PathTracker:collect(b, is_new)
    if self.overflow then return end
    local will_collect = false
    if is_new then
        self.collecting_depth = 1
        self.collect_buffer = ""
        will_collect = true
    elseif self.collecting_depth > 0 and not self.done then
        will_collect = true
    end
    if will_collect then
        self.collect_buffer = self.collect_buffer .. b
        if self.max_value_length > 0 and #self.collect_buffer > self.max_value_length then
            self.overflow = true
        end
    end
end

function PathTracker:collect_start_marker(b)
    -- Only the last byte of `b` (the structural char itself, not any preceding ':')
    local last = string.sub(b, #b, #b)
    self:collect(last, true)
end

function PathTracker:is_array_of_interest()
    local k = self.current_key or ""
    self.current_key = nil

    if self.skipped_depth > 0 then
        self.skipped_depth = self.skipped_depth + 1
    elseif self.matched_depth < #self.path_vector
        and k == self.path_vector[self.matched_depth + 1]
    then
        if self.matched_depth == #self.path_vector - 1 then
            return true  -- path ends here — collect whole array
        else
            self.matched_depth = self.matched_depth + 1
            self.array_nesting = self.array_nesting + 1
        end
    else
        self.skipped_depth = self.skipped_depth + 1
    end
    return false
end

function PathTracker:is_object_of_interest()
    if self.skipped_depth > 0 then
        self.skipped_depth = self.skipped_depth + 1
    elseif self.array_nesting > 0 then
        self.array_nesting = self.array_nesting + 1
    else
        local k = self.current_key or ""
        self.current_key = nil
        if self:match_key(k) then
            return true
        end
    end
    return false
end

function PathTracker:will_collect()
    if self.skipped_depth == 0
        and #self.path_vector > 0
        and self.matched_depth == #self.path_vector - 1
    then
        local k = self.current_key
        if k and k == self.path_vector[self.matched_depth + 1] then
            return true
        end
    end
    return false
end

function PathTracker:move_collect_pointers(is_start_object, is_end_object, is_start_array, is_end_array)
    if is_start_object or is_start_array then
        self.collecting_depth = self.collecting_depth + 1
    elseif is_end_object or is_end_array then
        self.collecting_depth = self.collecting_depth - 1
    end
end

function PathTracker:unwind(array_only)
    if self.skipped_depth > 0 then
        self.skipped_depth = self.skipped_depth - 1
    elseif self.array_nesting > 0 then
        self.array_nesting = self.array_nesting - 1
        if array_only and self.array_nesting == 0 and self.matched_depth > 0 then
            self.matched_depth = self.matched_depth - 1
        end
    elseif not array_only and self.matched_depth > 0 then
        self.matched_depth = self.matched_depth - 1
    end
end

function PathTracker:match_key(k)
    local pv = self.path_vector
    -- If k is empty but path component is not, don't match
    if k == "" and self.matched_depth < #pv and pv[self.matched_depth + 1] ~= "" then
        return false
    end
    if self.matched_depth < #pv and k == pv[self.matched_depth + 1] then
        if self.matched_depth == #pv - 1 then
            -- Terminal: start collecting
            self.collecting_depth = 1
            self.collect_buffer = ""
            return true
        else
            self.matched_depth = self.matched_depth + 1
            return false
        end
    else
        self.skipped_depth = self.skipped_depth + 1
        return false
    end
end

function PathTracker:finish()
    if not self.done and not self.overflow and #self.collect_buffer > 0 then
        self.done = true
    elseif self.overflow then
        self:reset(true)
    end
end

function PathTracker:has_data()
    return #self.collect_buffer > 0
end

-- Returns a decoded Lua value, or nil.
function PathTracker:get_value()
    local buf = self.collect_buffer
    if #buf == 0 then return nil end

    local first = string.byte(buf, 1)
    if first == 0x7B or first == 0x5B then
        -- Object or array — decode via event generator
        return decode_json_value(buf)
    else
        -- Scalar: try as number first, then boolean/null, then string
        local v = decode_json_value(buf)
        if v ~= nil then return v end
        -- Fall back: treat raw bytes as a plain string
        return buf
    end
end

function PathTracker:reset(overflow)
    self.matched_depth    = 0
    self.array_nesting    = 0
    self.skipped_depth    = 0
    self.current_key      = nil
    self.done             = false
    self.collecting_depth = 0
    self.collect_buffer   = ""
    self.overflow         = overflow
end

-- ── ChunkParser ───────────────────────────────────────────────────────────

local ChunkParser = {}
ChunkParser.__index = ChunkParser

function ChunkParser.new_json_parser(path_map)
    local self = setmetatable({
        scratch_buffer    = "",
        json_parser       = parser.JSONEventGenerator.new(),
        stop_at_first_match = true,
        tracked_fields    = {},
        matches_found     = {},
        done_fields       = {},
        overflowed_fields = {},
        json_depth        = 0,
        json_started      = false,
        end_of_json       = false,
        end_of_stream     = false,
        short_circuit     = false,
    }, ChunkParser)

    -- path_map: { [path_string] = { output_key_or_nil, max_size } }
    for path, opts in pairs(path_map) do
        self:add_search_field(path, opts[1], opts[2])
    end

    return self
end

function ChunkParser:add_search_field(json_path, output_key, max_size)
    self.tracked_fields[json_path] = PathTracker.new(json_path, output_key, max_size)
end

-- Process multiple chunks; last one is marked end-of-stream.
function ChunkParser:process_chunks(chunks)
    local total = #chunks
    for i, chunk in ipairs(chunks) do
        self:process_chunk(chunk, i == total)
        if self:is_all_found() then break end
    end
end

function ChunkParser:process_chunk(chunk, end_of_stream)
    self.scratch_buffer = self.scratch_buffer .. chunk
    local cursor = 0

    while true do
        local slice = string.sub(self.scratch_buffer, cursor + 1)
        local w = self.json_parser:next_event(slice, end_of_stream)

        local event_start = cursor
        cursor = cursor + w.consumed_bytes
        local b = string.sub(self.scratch_buffer, event_start + 1, cursor)

        local ev = w.event

        if ev == nil then
            if w.error then break end
            -- No event: bytes consumed (e.g. ':' or ',') — feed to collectors
            if w.consumed_bytes > 0 then
                self:feed_trackers(b)
            end
            break
        end

        self.json_started = true

        local ek = ev.kind

        if ek == parser.EVENT_EOF then
            break
        end

        local obj_key      = (ek == parser.EVENT_OBJECT_KEY) and ev.value or nil
        local is_start_obj = (ek == parser.EVENT_START_OBJECT)
        local is_end_obj   = (ek == parser.EVENT_END_OBJECT)
        local is_start_arr = (ek == parser.EVENT_START_ARRAY)
        local is_end_arr   = (ek == parser.EVENT_END_ARRAY)

        local leaf_val = nil
        if ek == parser.EVENT_STRING or ek == parser.EVENT_NUMBER then
            leaf_val = ev.value
        elseif ek == parser.EVENT_BOOLEAN then
            leaf_val = ev.value and "true" or "false"
        end

        if is_start_obj or is_start_arr then
            self.json_depth = self.json_depth + 1
        elseif is_end_obj or is_end_arr then
            self.json_depth = self.json_depth - 1
        end

        for _, tracker in pairs(self.tracked_fields) do
            if tracker.done then goto next_tracker end

            if tracker:is_collecting() then
                tracker:collect(b, false)
                if tracker.overflow then
                    self.overflowed_fields[tracker.path] = true
                    tracker:reset(true)
                end
                tracker:move_collect_pointers(is_start_obj, is_end_obj, is_start_arr, is_end_arr)
                if not tracker:is_collecting() then
                    tracker:finish()
                    self:end_tracker(tracker)
                end
                goto next_tracker
            end

            if obj_key ~= nil then
                if not tracker:is_skipping() then
                    tracker:set_current_key(obj_key)
                end
            elseif is_start_obj then
                if tracker:is_object_of_interest() then
                    tracker:collect_start_marker(b)
                end
            elseif is_end_obj then
                tracker:unwind(false)
            elseif is_start_arr then
                if tracker:is_array_of_interest() then
                    tracker:collect_start_marker(b)
                end
            elseif is_end_arr then
                tracker:unwind(true)
            elseif leaf_val ~= nil then
                if tracker:will_collect() then
                    tracker:collect(leaf_val, true)
                    tracker:finish()
                    self:end_tracker(tracker)
                end
            end

            ::next_tracker::
        end

        if self:is_all_done() then
            self.short_circuit = true
            break
        end
    end

    -- Drain consumed bytes from scratch_buffer
    self.scratch_buffer = string.sub(self.scratch_buffer, cursor + 1)

    if self.json_started and self.json_depth == 0 then
        self.end_of_json = true
    end
    self.end_of_stream = end_of_stream

    if end_of_stream or self.short_circuit or self.end_of_json then
        self:end_tracking()
    end
end

function ChunkParser:end_tracking()
    for _, tracker in pairs(self.tracked_fields) do
        tracker:finish()
        self:end_tracker(tracker)
    end
end

function ChunkParser:end_tracker(tracker)
    if not tracker.overflow then
        local v = tracker:get_value()
        if v ~= nil then
            local key = tracker.output_key or tracker.path
            self.matches_found[key] = v
            self.done_fields[tracker.path] = true
        end
    else
        self.overflowed_fields[tracker.path] = true
    end
end

function ChunkParser:feed_trackers(b)
    for _, tracker in pairs(self.tracked_fields) do
        tracker:collect(b, false)
        if tracker.overflow then
            self.overflowed_fields[tracker.path] = true
            tracker:reset(true)
        end
    end
end

function ChunkParser:is_all_done()
    if not self.stop_at_first_match then return false end
    for _, t in pairs(self.tracked_fields) do
        if not t.done and not t.overflow then return false end
    end
    return true
end

function ChunkParser:is_all_found()
    local tracked = 0
    for _ in pairs(self.tracked_fields) do tracked = tracked + 1 end
    local found = 0
    for _ in pairs(self.done_fields) do found = found + 1 end
    local over = 0
    for _ in pairs(self.overflowed_fields) do over = over + 1 end
    return tracked == found + over
end

function ChunkParser:get_field(name)
    local t = self.tracked_fields[name]
    assert(t, "field not found: " .. tostring(name))
    return t
end

function ChunkParser:get_matches()
    return self.matches_found
end

-- Returns a JSON string of matches_found.
function ChunkParser:get_result_json()
    local parts = {}
    local keys = {}
    for k in pairs(self.matches_found) do keys[#keys+1] = k end
    table.sort(keys)
    for _, k in ipairs(keys) do
        parts[#parts+1] = encode_json_value(k) .. ":" .. encode_json_value(self.matches_found[k])
    end
    return "{" .. table.concat(parts, ",") .. "}"
end

M.ChunkParser = ChunkParser
M.PathTracker = PathTracker

return M
