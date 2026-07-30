-- test_helpers.lua
-- Mirrors rust/tests/test_helpers.rs
-- Utility functions for building test JSON and exercising the chunk parser.

local parser    = require("parser")
local cp_module = require("chunk_parser")

local M = {}

-- ── LCG random number generator ────────────────────────────────────────────
-- 32-bit Knuth LCG — works in LuaJIT (doubles) and Lua 5.3+ (integers).
-- Result is always in [0, 2^32-1], so no sign-masking needed.
function M.lcg_next(state)
    return (state * 1664525 + 1013904223) % 0x100000000
end

-- ── String helpers ──────────────────────────────────────────────────────────

function M.rep(ch, len)
    return string.rep(ch, len)
end

-- Build a value string: characters repeated, optionally quoted.
function M.text_value(ch, value_len, quote)
    local inner = M.rep(ch, value_len)
    return quote .. inner .. quote
end

-- ── JSON building helpers ───────────────────────────────────────────────────

function M.str_array(ch, item_count, value_len)
    local items = {}
    for _ = 1, item_count do
        items[#items + 1] = M.text_value(ch, value_len, '"')
    end
    return "[ " .. table.concat(items, ",") .. " ]"
end

function M.obj_array(kch, vch, item_count, value_len)
    local items = {}
    for _ = 1, item_count do
        items[#items + 1] = string.format('{"name":%s,"value":%s}',
            M.text_value(kch, value_len, '"'),
            M.text_value(vch, value_len, '"'))
    end
    return "[ " .. table.concat(items, ",") .. " ]"
end

-- Build flat scalar fields: "field_0": "aaa...", "field_1": "bbb...", ...
-- Populates all_expected[field_name] = decoded_value.
function M.build_flat_fields(field_count, value_len, all_expected)
    local parts = {}
    for i = 0, field_count - 1 do
        local ch = string.char(string.byte('a') + (i % 26))
        local key = string.format("field_%d", i)
        local value = M.rep(ch, value_len)
        parts[#parts + 1] = string.format('"%s": "%s"', key, value)
        if all_expected then
            -- Stored as a plain Lua string (the JSON string value)
            all_expected[key] = value
        end
    end
    return table.concat(parts, ",") .. ","
end

-- Build the small JSON structure (matches Rust's build_small_json exactly).
-- Populates all_expected with paths → values.
function M.build_small_json(field_count, value_len, all_expected)
    local author    = M.text_value('A', value_len, ' ')
    local version   = M.text_value('B', value_len, ' ')
    local views     = M.obj_array('C', 'D', field_count, value_len)
    local regions   = M.str_array('E', field_count, value_len)
    local locale    = M.text_value('F', value_len, ' ')
    local name      = M.text_value('G', value_len, ' ')
    local tags      = M.str_array('H', field_count, value_len)
    local items     = M.obj_array('I', 'J', field_count, value_len)
    local key       = M.text_value('K', value_len, ' ')
    local value1    = M.text_value('L', value_len, ' ')
    local value2    = M.text_value('M', value_len, ' ')
    local signature = M.str_array('N', field_count, value_len)
    local timestamp = 123456789

    local json = string.format(
        '{'..
        ' "metadata" :  {'..
        '   ""  :  "%s"  ,'..
        '   "author"  :  "%s"  ,'..
        '"version":"%s",'..
        '"stats":{'..
        '"views":%s,'..
        ' "details" :   {  '..
        '  "regions"   :   %s,'..
        '  "locale"   :  "%s"  '..
        '}},'..
        '"name": "%s"'..
        '},'..
        '"":%s,'..
        '"items":%s,'..
        '"config":{'..
        '"key": "%s",'..
        '"values":{'..
        '"value1": "%s",'..
        '"value2": "%s"'..
        '}},'..
        '"signature":%s,'..
        '"timestamp":%d'..
        '}',
        author, author, version,
        views, regions, locale, name,
        tags, items, key, value1, value2,
        signature, timestamp
    )

    if all_expected then
        -- Scalar strings: stored as raw leaf values (what the parser returns)
        -- The parser strips JSON quotes, so we store the unquoted content.
        -- text_value uses space as quote, so " FFF " → parser returns " FFF ".
        all_expected["metadata."]                     = author
        all_expected["metadata.author"]               = author
        all_expected["metadata.version"]              = version
        -- Container values: stored as raw JSON strings (parser decodes to table)
        all_expected["metadata.stats.views"]          = views
        all_expected["metadata.stats.details.regions"]= regions
        all_expected["metadata.stats.details.locale"] = locale
        all_expected["metadata.name"]                 = name
        all_expected[""]                              = tags
        all_expected["items"]                         = items
        all_expected["config.key"]                    = key
        all_expected["config.values.value1"]          = value1
        all_expected["config.values.value2"]          = value2
        all_expected["signature"]                     = signature
        -- Numbers stored as Lua numbers (parser returns tonumber())
        all_expected["timestamp"]                     = timestamp
    end

    return json
end

-- Build the large JSON (flat fields + small JSON structure).
function M.build_large_json(field_count, value_len, all_expected)
    local flat = M.build_flat_fields(field_count, value_len, all_expected)
    local small = M.build_small_json(field_count, value_len, all_expected)
    -- Remove outer braces from small JSON to embed it
    small = string.sub(small, 2, #small - 1)  -- strip leading '{' and trailing '}'
    return "{" .. flat .. small .. "}"
end

-- Build invalid JSON (small JSON + garbage appended).
function M.build_invalid_json(field_count, value_len, all_expected)
    local json = M.build_small_json(field_count, value_len, all_expected)
    return json .. "-------" .. "+++++++"
end

-- Build plain text input (not valid JSON).
local function build_text_section(field_count, value_len)
    local parts = {}
    for i = 0, field_count - 1 do
        local ch = string.char(string.byte('a') + (i % 26))
        local key = string.format("field_%d", i)
        local value = M.rep(ch, value_len)
        parts[#parts + 1] = string.format(" %s  %s ", key, value)
    end
    return table.concat(parts, " ")
end

function M.build_text_input(field_count, value_len)
    local third = math.floor(field_count / 3)
    return build_text_section(third, value_len)
        .. "-------"
        .. build_text_section(third, value_len)
        .. "+++++++"
        .. build_text_section(third, value_len)
end

-- ── Chunk splitting ─────────────────────────────────────────────────────────

-- Split `bytes` (a string) into random-sized chunks [min, max).
-- If split_random_keys is true, occasionally splits in the middle of a key.
-- seed is the LCG seed (integer).
function M.random_chunks(bytes, min_size, max_size, seed, split_random_keys, keys)
    local chunks = {}
    local pos = 1  -- 1-based into bytes string
    local rng = seed
    local last_split = false
    local len = #bytes

    while pos <= len do
        local range = max_size - min_size
        rng = M.lcg_next(rng)
        -- rng is always in [0, 2^32-1] so modulo is always non-negative
        local size = min_size + (rng % math.max(range, 1))
        local end_pos = math.min(pos + size - 1, len)
        local chunk = string.sub(bytes, pos, end_pos)

        local did_split = false
        if split_random_keys and keys and #keys > 0 and not last_split then
            last_split = true
            for _, key in ipairs(keys) do
                if key ~= "" then
                    local ki = string.find(chunk, key, 1, true)
                    if ki then
                        local mid = ki + math.floor(#key / 2)
                        if mid < #chunk then
                            chunks[#chunks + 1] = string.sub(chunk, 1, mid)
                            chunks[#chunks + 1] = string.sub(chunk, mid + 1)
                            did_split = true
                            break
                        end
                    end
                end
            end
        else
            if split_random_keys then
                last_split = false
            end
        end

        if not did_split then
            chunks[#chunks + 1] = chunk
        end

        pos = end_pos + 1
    end

    return chunks
end

-- ── Expected value extraction ───────────────────────────────────────────────

-- Extract the JSON value at a dot-path from a JSON string using the event generator.
-- path_parts is a list of string keys, e.g. {"metadata", "stats", "locale"}.
-- Returns a Lua value (via decode_json_value), or nil.
function M.extract_json_value(json_str, path_parts)
    if #path_parts == 0 then return nil end

    local gen = parser.JSONEventGenerator.new()
    local cursor = 0
    local len = #json_str

    local pending_key    = nil
    local matched_depth  = 0
    local skipped_depth  = 0
    local collecting_depth = 0
    local collect_start  = 0  -- 0-based byte position of start of collected region

    while cursor <= len do
        local slice = string.sub(json_str, cursor + 1)
        local w = gen:next_event(slice, true)
        cursor = cursor + w.consumed_bytes

        if w.event == nil then
            if w.consumed_bytes == 0 then break end
        end

        if w.error and not w.event then break end

        local ev = w.event
        if ev == nil then break end
        local ek = ev.kind

        if ek == parser.EVENT_EOF then break end

        if ek == parser.EVENT_OBJECT_KEY then
            if skipped_depth == 0 and collecting_depth == 0 then
                pending_key = ev.value
            end

        elseif ek == parser.EVENT_START_OBJECT then
            if collecting_depth > 0 then
                collecting_depth = collecting_depth + 1
            else
                local k = pending_key or ""
                pending_key = nil
                if skipped_depth > 0 then
                    skipped_depth = skipped_depth + 1
                elseif k == "" then
                    -- root object — transparent
                elseif matched_depth < #path_parts and k == path_parts[matched_depth + 1] then
                    if matched_depth == #path_parts - 1 then
                        collecting_depth = 1
                        collect_start = cursor - 1  -- points at '{'
                    else
                        matched_depth = matched_depth + 1
                    end
                else
                    skipped_depth = skipped_depth + 1
                end
            end

        elseif ek == parser.EVENT_END_OBJECT then
            if collecting_depth > 0 then
                collecting_depth = collecting_depth - 1
                if collecting_depth == 0 then
                    local raw = string.sub(json_str, collect_start + 1, cursor)
                    return cp_module.decode_json_value(raw)
                end
            else
                if skipped_depth > 0 then
                    skipped_depth = skipped_depth - 1
                elseif matched_depth > 0 then
                    matched_depth = matched_depth - 1
                end
                pending_key = nil
            end

        elseif ek == parser.EVENT_START_ARRAY then
            if collecting_depth > 0 then
                collecting_depth = collecting_depth + 1
            else
                local k = pending_key
                pending_key = nil
                if skipped_depth > 0 then
                    skipped_depth = skipped_depth + 1
                elseif matched_depth < #path_parts
                    and k ~= nil and k == path_parts[matched_depth + 1]
                    and matched_depth == #path_parts - 1
                then
                    collecting_depth = 1
                    collect_start = cursor - 1  -- points at '['
                else
                    skipped_depth = skipped_depth + 1
                end
            end

        elseif ek == parser.EVENT_END_ARRAY then
            if collecting_depth > 0 then
                collecting_depth = collecting_depth - 1
                if collecting_depth == 0 then
                    local raw = string.sub(json_str, collect_start + 1, cursor)
                    return cp_module.decode_json_value(raw)
                end
            elseif skipped_depth > 0 then
                skipped_depth = skipped_depth - 1
            end

        elseif ek == parser.EVENT_STRING then
            if collecting_depth == 0 and skipped_depth == 0
                and matched_depth == #path_parts - 1
                and pending_key ~= nil and pending_key == path_parts[matched_depth + 1]
            then
                return ev.value  -- plain Lua string
            end
            if collecting_depth == 0 then pending_key = nil end

        elseif ek == parser.EVENT_NUMBER then
            if collecting_depth == 0 and skipped_depth == 0
                and matched_depth == #path_parts - 1
                and pending_key ~= nil and pending_key == path_parts[matched_depth + 1]
            then
                return tonumber(ev.value)
            end
            if collecting_depth == 0 then pending_key = nil end

        elseif ek == parser.EVENT_BOOLEAN then
            if collecting_depth == 0 and skipped_depth == 0
                and matched_depth == #path_parts - 1
                and pending_key ~= nil and pending_key == path_parts[matched_depth + 1]
            then
                return ev.value
            end
            if collecting_depth == 0 then pending_key = nil end

        else
            if collecting_depth == 0 then pending_key = nil end
        end
    end

    return nil
end

-- Build expected results from path_map + all_expected.
-- path_map: { [path] = { output_key_or_nil, max_size } }
-- all_expected: { [path] = raw_value }
function M.build_expected(path_map, all_expected)
    local expected = {}
    for path, opts in pairs(path_map) do
        local v = all_expected[path]
        if v ~= nil then
            local out_key = opts[1] or path
            expected[out_key] = v
        end
    end
    return expected
end

-- Build expected with the chunk index where the value first became readable.
function M.build_expected_with_pos(path_map, chunks)
    local expected = {}
    for json_path, opts in pairs(path_map) do
        local parts = {}
        for part in (json_path .. "."):gmatch("([^.]*)%.") do
            parts[#parts + 1] = part
        end
        local accumulated = ""
        for i, chunk in ipairs(chunks) do
            accumulated = accumulated .. chunk
            local v = M.extract_json_value(accumulated, parts)
            if v ~= nil then
                local out_key = opts[1] or json_path
                expected[out_key] = { i, v }
                break
            end
        end
    end
    return expected
end

-- Get all field name components for chunk-splitting heuristic.
function M.get_expected_field_names(path_map)
    local seen = {}
    local names = {}
    for path in pairs(path_map) do
        for part in (path .. "."):gmatch("([^.]*)%.") do
            if part ~= "" and not seen[part] then
                seen[part] = true
                names[#names + 1] = part
            end
        end
    end
    return names
end

-- Feed chunks to the parser; return the 1-based index of the chunk where
-- all targets were found, or nil.
function M.feed_chunks_to_parser(cp, chunks)
    local total = #chunks
    for i, chunk in ipairs(chunks) do
        cp:process_chunk(chunk, i == total)
        if cp:is_all_found() then
            return i
        end
    end
    return nil
end

-- ── Deep equality ────────────────────────────────────────────────────────────

-- Compare two Lua values that may have come from JSON decoding.
-- Strings from all_expected are raw JSON-string contents (no quotes),
-- but extract_json_value also returns the raw string content — so comparison
-- is direct string equality for strings.
function M.values_equal(a, b)
    if type(a) ~= type(b) then
        -- Allow number/string comparison for numeric values
        if type(a) == "number" and type(b) == "string" then
            return tostring(a) == b or a == tonumber(b)
        elseif type(a) == "string" and type(b) == "number" then
            return a == tostring(b) or tonumber(a) == b
        end
        return false
    end
    if type(a) == "table" then
        if a == cp_module.NULL and b == cp_module.NULL then return true end
        for k, v in pairs(a) do
            if k ~= "is_array" then
                if not M.values_equal(v, b[k]) then return false end
            end
        end
        for k in pairs(b) do
            if k ~= "is_array" and a[k] == nil then return false end
        end
        return true
    end
    return a == b
end

return M
