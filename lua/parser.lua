-- parser.lua
-- Low-level JSON lexer and event generator.
-- Ported from the Rust implementation in rust/src/parser.rs.
--
-- All byte offsets are 0-based internally (matching Rust). Lua string indexing
-- is 1-based, so every string.byte / string.sub call adds 1.

local M = {}

local MAX_STATE_STACK_SIZE = 65536

-- ── Event kind constants ───────────────────────────────────────────────────

M.EVENT_STRING       = "String"
M.EVENT_NUMBER       = "Number"
M.EVENT_BOOLEAN      = "Boolean"
M.EVENT_NULL         = "Null"
M.EVENT_START_ARRAY  = "StartArray"
M.EVENT_END_ARRAY    = "EndArray"
M.EVENT_START_OBJECT = "StartObject"
M.EVENT_END_OBJECT   = "EndObject"
M.EVENT_OBJECT_KEY   = "ObjectKey"
M.EVENT_EOF          = "Eof"

-- ── State / token constants (internal) ────────────────────────────────────

local STATE_OBJECT_KEY          = 1
local STATE_OBJECT_KEY_OR_END   = 2
local STATE_OBJECT_COLON        = 3
local STATE_OBJECT_VALUE        = 4
local STATE_OBJECT_COMMA_OR_END = 5
local STATE_ARRAY_VALUE         = 6
local STATE_ARRAY_VALUE_OR_END  = 7
local STATE_ARRAY_COMMA_OR_END  = 8

local TOKEN_OPENING_SQUARE = 1
local TOKEN_CLOSING_SQUARE = 2
local TOKEN_OPENING_CURLY  = 3
local TOKEN_CLOSING_CURLY  = 4
local TOKEN_COMMA          = 5
local TOKEN_COLON          = 6
local TOKEN_STRING         = 7
local TOKEN_NUMBER         = 8
local TOKEN_TRUE           = 9
local TOKEN_FALSE          = 10
local TOKEN_NULL_TOK       = 11
local TOKEN_EOF            = 12

-- ── Helpers ────────────────────────────────────────────────────────────────

-- Encode a Unicode code point as a UTF-8 string.
local function utf8_encode(cp)
    if cp < 0 then
        return "\xEF\xBF\xBD" -- U+FFFD replacement character
    elseif cp <= 0x7F then
        return string.char(cp)
    elseif cp <= 0x7FF then
        return string.char(
            0xC0 + math.floor(cp / 64),
            0x80 + (cp % 64)
        )
    elseif cp <= 0xFFFF then
        return string.char(
            0xE0 + math.floor(cp / 4096),
            0x80 + math.floor((cp % 4096) / 64),
            0x80 + (cp % 64)
        )
    elseif cp <= 0x10FFFF then
        return string.char(
            0xF0 + math.floor(cp / 262144),
            0x80 + math.floor((cp % 262144) / 4096),
            0x80 + math.floor((cp % 4096) / 64),
            0x80 + (cp % 64)
        )
    else
        return "\xEF\xBF\xBD"
    end
end

-- Read 4 hex chars from buf starting at 1-based position `pos`.
-- Returns code_point (integer) or nil + error message.
local function read_hexa_char(buf, pos)
    local value = 0
    for i = pos, pos + 3 do
        local b = string.byte(buf, i)
        if not b then
            return nil, "Unexpected end in unicode escape sequence"
        end
        local digit
        if b >= 0x30 and b <= 0x39 then     -- '0'-'9'
            digit = b - 0x30
        elseif b >= 0x61 and b <= 0x66 then  -- 'a'-'f'
            digit = b - 0x61 + 10
        elseif b >= 0x41 and b <= 0x46 then  -- 'A'-'F'
            digit = b - 0x41 + 10
        else
            return nil, string.format("Unexpected character in a unicode escape: '%s'", string.char(b))
        end
        value = value * 16 + digit
    end
    return value, nil
end

-- Count leading ASCII digits in buf starting at 1-based `pos`.
-- Returns count, or nil if we ran out of buffer without hitting a non-digit
-- (meaning we need more data).
local function read_digits(buf, pos, is_ending)
    local count = 0
    local len = #buf
    local i = pos
    while i <= len do
        local b = string.byte(buf, i)
        if b >= 0x30 and b <= 0x39 then  -- '0'-'9'
            count = count + 1
            i = i + 1
        else
            return count
        end
    end
    -- Reached end of buffer
    if is_ending then
        return count
    end
    return nil  -- need more data
end

-- ── JSONLexer ──────────────────────────────────────────────────────────────

local JSONLexer = {}
JSONLexer.__index = JSONLexer

function JSONLexer.new()
    return setmetatable({
        file_offset            = 0,  -- global byte offset (0-based)
        file_line              = 0,
        file_start_of_last_line  = 0,
        file_start_of_last_token = 0,
        is_start               = true,
    }, JSONLexer)
end

-- Build a syntax error table.
-- start_off, end_off are 0-based absolute offsets.
function JSONLexer:syntax_error(start_off, end_off, message)
    local s_off = math.max(start_off, self.file_start_of_last_line)
    return {
        is_error = true,
        message  = message,
        location = {
            start = {
                line   = self.file_line,
                column = s_off - self.file_start_of_last_line,
                offset = s_off,
            },
            finish = {
                line   = self.file_line,
                column = end_off - self.file_start_of_last_line,
                offset = end_off,
            },
        },
    }
end

-- Format a syntax error as a human-readable string.
function M.error_to_string(err)
    local s = err.location.start
    local e = err.location.finish
    if s.offset + 1 >= e.offset then
        return string.format("Parser error at line %d column %d: %s",
            s.line + 1, s.column + 1, err.message)
    elseif s.line == e.line then
        return string.format("Parser error at line %d between columns %d and column %d: %s",
            s.line + 1, s.column + 1, e.column + 1, err.message)
    else
        return string.format("Parser error between line %d column %d and line %d column %d: %s",
            s.line + 1, s.column + 1, e.line + 1, e.column + 1, err.message)
    end
end

-- Read the next token from `buf` (a Lua string).
-- `buf` starts at the current file_offset position in the stream.
-- Returns: token_table, error_table, ok
--   token_table: { kind=..., value=... } or nil
--   error_table: syntax error or nil
--   ok: false means "need more data" (return nil,nil,false); true means we produced something
function JSONLexer:read_next_token(buf, is_ending)
    -- Strip BOM at the very beginning of the stream
    if self.is_start then
        if #buf < 3 and not is_ending then
            return nil, nil, false
        end
        self.is_start = false
        if #buf >= 3
            and string.byte(buf, 1) == 0xEF
            and string.byte(buf, 2) == 0xBB
            and string.byte(buf, 3) == 0xBF
        then
            buf = string.sub(buf, 4)
            self.file_offset = self.file_offset + 3
        end
    end

    -- Skip whitespace
    local i = 1  -- 1-based index into buf
    while i <= #buf do
        local b = string.byte(buf, i)
        if b == 0x20 or b == 0x09 then  -- space, tab
            i = i + 1
        elseif b == 0x0A then  -- '\n'
            i = i + 1
            self.file_line = self.file_line + 1
            self.file_start_of_last_line = self.file_offset + i - 1
        elseif b == 0x0D then  -- '\r'
            i = i + 1
            local nb = string.byte(buf, i)
            if nb == 0x0A then
                i = i + 1
            elseif not is_ending then
                i = i - 1
                self.file_offset = self.file_offset + i - 1
                return nil, nil, false
            end
            self.file_line = self.file_line + 1
            self.file_start_of_last_line = self.file_offset + i - 1
        else
            break
        end
    end
    self.file_offset = self.file_offset + i - 1
    buf = string.sub(buf, i)
    self.file_start_of_last_token = self.file_offset

    if is_ending and #buf == 0 then
        return { kind = TOKEN_EOF }, nil, true
    end

    if #buf == 0 then
        return nil, nil, false
    end

    local c = string.byte(buf, 1)

    if c == 0x7B then  -- '{'
        self.file_offset = self.file_offset + 1
        return { kind = TOKEN_OPENING_CURLY }, nil, true
    elseif c == 0x7D then  -- '}'
        self.file_offset = self.file_offset + 1
        return { kind = TOKEN_CLOSING_CURLY }, nil, true
    elseif c == 0x5B then  -- '['
        self.file_offset = self.file_offset + 1
        return { kind = TOKEN_OPENING_SQUARE }, nil, true
    elseif c == 0x5D then  -- ']'
        self.file_offset = self.file_offset + 1
        return { kind = TOKEN_CLOSING_SQUARE }, nil, true
    elseif c == 0x2C then  -- ','
        self.file_offset = self.file_offset + 1
        return { kind = TOKEN_COMMA }, nil, true
    elseif c == 0x3A then  -- ':'
        self.file_offset = self.file_offset + 1
        return { kind = TOKEN_COLON }, nil, true
    elseif c == 0x22 then  -- '"'
        return self:read_string(buf)
    elseif c == 0x74 then  -- 't'
        return self:read_constant(buf, is_ending, "true", TOKEN_TRUE)
    elseif c == 0x66 then  -- 'f'
        return self:read_constant(buf, is_ending, "false", TOKEN_FALSE)
    elseif c == 0x6E then  -- 'n'
        return self:read_constant(buf, is_ending, "null", TOKEN_NULL_TOK)
    elseif c == 0x2D or (c >= 0x30 and c <= 0x39) then  -- '-' or '0'-'9'
        return self:read_number(buf, is_ending)
    else
        self.file_offset = self.file_offset + 1
        local msg
        if c < 128 then
            msg = string.format("Unexpected char: '%s'", string.char(c))
        else
            msg = string.format("Unexpected byte: \\x%X", c)
        end
        return nil, self:syntax_error(self.file_offset - 1, self.file_offset, msg), true
    end
end

function JSONLexer:read_string(buf)
    -- buf[1] == '"', collect until closing '"'
    local has_escapes = false
    local result_parts = {}
    local plain_start = 2  -- start of plain-text segment (1-based)
    local nbo = 2  -- next_byte_offset (1-based into buf)

    while nbo <= #buf do
        local b = string.byte(buf, nbo)

        if b == 0x22 then  -- '"' end of string
            local final_str
            if not has_escapes then
                -- Entire string is plain ASCII/UTF-8 (or simple UTF-8)
                final_str = string.sub(buf, 2, nbo - 1)
            else
                -- Flush remaining plain bytes
                if plain_start < nbo then
                    result_parts[#result_parts + 1] = string.sub(buf, plain_start, nbo - 1)
                end
                final_str = table.concat(result_parts)
            end
            self.file_offset = self.file_offset + nbo  -- skip past closing '"'
            return { kind = TOKEN_STRING, value = final_str }, nil, true

        elseif b == 0x5C then  -- '\'  escape
            if not has_escapes then
                -- Flush plain bytes collected so far
                if plain_start < nbo then
                    result_parts[#result_parts + 1] = string.sub(buf, plain_start, nbo - 1)
                end
                has_escapes = true
            end
            nbo = nbo + 1
            if nbo > #buf then return nil, nil, false end

            local esc = string.byte(buf, nbo)
            if esc == 0x22 then       -- \"
                result_parts[#result_parts + 1] = '"';  nbo = nbo + 1
            elseif esc == 0x5C then   -- \\
                result_parts[#result_parts + 1] = '\\'; nbo = nbo + 1
            elseif esc == 0x2F then   -- \/
                result_parts[#result_parts + 1] = '/';  nbo = nbo + 1
            elseif esc == 0x62 then   -- \b
                result_parts[#result_parts + 1] = '\x08'; nbo = nbo + 1
            elseif esc == 0x66 then   -- \f
                result_parts[#result_parts + 1] = '\x0C'; nbo = nbo + 1
            elseif esc == 0x6E then   -- \n
                result_parts[#result_parts + 1] = '\n'; nbo = nbo + 1
            elseif esc == 0x72 then   -- \r
                result_parts[#result_parts + 1] = '\r'; nbo = nbo + 1
            elseif esc == 0x74 then   -- \t
                result_parts[#result_parts + 1] = '\t'; nbo = nbo + 1
            elseif esc == 0x75 then   -- \uXXXX
                nbo = nbo + 1
                if nbo + 3 > #buf then return nil, nil, false end
                local cp, hex_err = read_hexa_char(buf, nbo)
                local err_obj = nil
                if not cp then
                    local pos = self.file_offset + nbo - 1
                    err_obj = self:syntax_error(pos, pos + 4, hex_err)
                    result_parts[#result_parts + 1] = "\xEF\xBF\xBD"  -- U+FFFD
                    nbo = nbo + 4
                elseif cp >= 0xD800 and cp <= 0xDBFF then
                    -- High surrogate
                    local high = cp
                    nbo = nbo + 4
                    if nbo + 5 > #buf then return nil, nil, false end
                    local b1 = string.byte(buf, nbo)
                    local b2 = string.byte(buf, nbo + 1)
                    if not (b1 == 0x5C and b2 == 0x75) then  -- not \u
                        local pos = self.file_offset + nbo - 1
                        if not err_obj then
                            err_obj = self:syntax_error(pos, pos + 6,
                                string.format("\\u%X is a high surrogate and should be followed by a low surrogate \\uXXXX", high))
                        end
                        nbo = nbo + 6
                        result_parts[#result_parts + 1] = "\xEF\xBF\xBD"
                    else
                        nbo = nbo + 2
                        if nbo + 3 > #buf then return nil, nil, false end
                        local low, low_err = read_hexa_char(buf, nbo)
                        nbo = nbo + 4
                        if not low then
                            local pos = self.file_offset + nbo - 5
                            if not err_obj then
                                err_obj = self:syntax_error(pos, pos + 4, low_err)
                            end
                            result_parts[#result_parts + 1] = "\xEF\xBF\xBD"
                        elseif low < 0xDC00 or low > 0xDFFF then
                            local pos = self.file_offset + nbo - 5
                            if not err_obj then
                                err_obj = self:syntax_error(pos, pos + 4,
                                    string.format("\\u%X is not a valid low surrogate", low))
                            end
                            result_parts[#result_parts + 1] = "\xEF\xBF\xBD"
                        else
                            -- & 0x03FF == % 0x0400 (mask low 10 bits); << 10 == * 0x0400
                            local full_cp = 0x10000 + ((high % 0x0400) * 0x0400) + (low % 0x0400)
                            result_parts[#result_parts + 1] = utf8_encode(full_cp)
                        end
                    end
                elseif cp >= 0xDC00 and cp <= 0xDFFF then
                    -- Standalone low surrogate
                    local pos = self.file_offset + nbo - 1
                    if not err_obj then
                        err_obj = self:syntax_error(pos, pos + 4,
                            string.format("\\u%X is not a valid high surrogate", cp))
                    end
                    result_parts[#result_parts + 1] = "\xEF\xBF\xBD"
                    nbo = nbo + 4
                else
                    result_parts[#result_parts + 1] = utf8_encode(cp)
                    nbo = nbo + 4
                end
                plain_start = nbo
                if err_obj then
                    -- Flush rest and return error
                    self.file_offset = self.file_offset + nbo - 1
                    return nil, err_obj, true
                end
                -- continue without advancing plain_start again (already set)
                goto continue
            else
                nbo = nbo + 1
                local pos = self.file_offset + nbo - 1
                local err_obj = self:syntax_error(pos - 2, pos,
                    string.format("'\\%s' is not a valid escape sequence", string.char(esc)))
                result_parts[#result_parts + 1] = "\xEF\xBF\xBD"
                plain_start = nbo
                -- Continue parsing; we report the error at end of string
                -- (matching Rust: error stored, string parsing continues)
                self.file_offset = self.file_offset + nbo - 1
                return nil, err_obj, true
            end
            plain_start = nbo

        elseif b <= 0x1F then  -- control character
            local pos = self.file_offset + nbo - 1
            local err_obj = self:syntax_error(pos, pos + 1,
                string.format("'%s' is not allowed in JSON strings", string.char(b)))
            nbo = nbo + 1
            -- Continue (like Rust: stores first error, keeps parsing)
            self.file_offset = self.file_offset + nbo - 1
            return nil, err_obj, true
        else
            if has_escapes then
                -- Plain byte inside an escape-mode string — accumulated lazily
            end
            nbo = nbo + 1
        end

        ::continue::
    end

    -- Ran out of buffer without finding closing '"'
    return nil, nil, false
end

function JSONLexer:read_constant(buf, is_ending, expected, token_kind)
    local elen = #expected
    if #buf >= elen and string.sub(buf, 1, elen) == expected then
        self.file_offset = self.file_offset + elen
        return { kind = token_kind }, nil, true
    end

    -- Count leading alphabetic chars
    local alpha_count = 0
    for i = 1, #buf do
        local b = string.byte(buf, i)
        if (b >= 0x61 and b <= 0x7A) or (b >= 0x41 and b <= 0x5A) then
            alpha_count = alpha_count + 1
        else
            break
        end
    end

    if alpha_count == #buf and not is_ending then
        return nil, nil, false  -- might be more to read
    end

    local read = math.max(1, alpha_count)
    local start_off = self.file_offset
    self.file_offset = self.file_offset + read
    local err = self:syntax_error(start_off, self.file_offset,
        string.format("%s expected", expected))
    return nil, err, true
end

function JSONLexer:read_number(buf, is_ending)
    local nbo = 1  -- 1-based into buf

    if string.byte(buf, nbo) == 0x2D then  -- '-'
        nbo = nbo + 1
    end

    if nbo > #buf then return nil, nil, false end

    local first = string.byte(buf, nbo)
    if first == 0x30 then  -- '0'
        nbo = nbo + 1
    elseif first >= 0x31 and first <= 0x39 then  -- '1'-'9'
        nbo = nbo + 1
        local cnt = read_digits(buf, nbo, is_ending)
        if cnt == nil then return nil, nil, false end
        nbo = nbo + cnt
    else
        nbo = nbo + 1
        self.file_offset = self.file_offset + nbo - 1
        local err = self:syntax_error(self.file_offset - 1, self.file_offset,
            string.format("A number is not allowed to start with '%s'", string.char(first)))
        return nil, err, true
    end

    -- Fractional part
    if nbo <= #buf and string.byte(buf, nbo) == 0x2E then  -- '.'
        nbo = nbo + 1
        if nbo > #buf then
            if not is_ending then return nil, nil, false end
            -- ending with just '.' — error handled below
        else
            local fc = string.byte(buf, nbo)
            nbo = nbo + 1
            if not (fc >= 0x30 and fc <= 0x39) then
                self.file_offset = self.file_offset + nbo - 1
                local err = self:syntax_error(self.file_offset - 1, self.file_offset,
                    string.format("A number fractional part must start with a digit and not '%s'", string.char(fc)))
                return nil, err, true
            end
            local cnt = read_digits(buf, nbo, is_ending)
            if cnt == nil then return nil, nil, false end
            nbo = nbo + cnt
        end
    elseif nbo > #buf and not is_ending then
        return nil, nil, false
    end

    -- Exponent part
    if nbo <= #buf then
        local ec = string.byte(buf, nbo)
        if ec == 0x65 or ec == 0x45 then  -- 'e' or 'E'
            nbo = nbo + 1
            if nbo > #buf then return nil, nil, false end
            local sign = string.byte(buf, nbo)
            if sign == 0x2D or sign == 0x2B then  -- '-' or '+'
                nbo = nbo + 1
                if nbo > #buf then return nil, nil, false end
                local dc = string.byte(buf, nbo)
                nbo = nbo + 1
                if not (dc >= 0x30 and dc <= 0x39) then
                    self.file_offset = self.file_offset + nbo - 1
                    local err = self:syntax_error(self.file_offset - 1, self.file_offset,
                        string.format("A number exponential part must contain at least a digit, '%s' found", string.char(dc)))
                    return nil, err, true
                end
            elseif sign >= 0x30 and sign <= 0x39 then
                nbo = nbo + 1
            else
                nbo = nbo + 1
                self.file_offset = self.file_offset + nbo - 1
                local err = self:syntax_error(self.file_offset - 1, self.file_offset,
                    string.format("A number exponential part must start with +, - or a digit, '%s' found", string.char(sign)))
                return nil, err, true
            end
            local cnt = read_digits(buf, nbo, is_ending)
            if cnt == nil then return nil, nil, false end
            nbo = nbo + cnt
        end
    elseif not is_ending then
        return nil, nil, false
    end

    local num_str = string.sub(buf, 1, nbo - 1)
    self.file_offset = self.file_offset + nbo - 1
    return { kind = TOKEN_NUMBER, value = num_str }, nil, true
end

-- ── JSONEventGenerator ─────────────────────────────────────────────────────

local JSONEventGenerator = {}
JSONEventGenerator.__index = JSONEventGenerator

function JSONEventGenerator.new()
    return setmetatable({
        lexer               = JSONLexer.new(),
        state_stack         = {},
        max_state_stack_size = MAX_STATE_STACK_SIZE,
        element_read        = false,
        buffered_event      = nil,
    }, JSONEventGenerator)
end

function JSONEventGenerator:with_max_stack_size(size)
    self.max_state_stack_size = size
    return self
end

-- Returns: { consumed_bytes=N, event=ev_or_nil, error=err_or_nil }
-- event is a table like { kind=EVENT_STRING, value="..." } etc.
-- error is a syntax error table or nil.
-- When event==nil and error==nil: need more data.
-- When event==nil and error~=nil: error (no event produced).
function JSONEventGenerator:next_event(buf, is_ending)
    -- Return buffered event first (no bytes consumed)
    if self.buffered_event then
        local ev = self.buffered_event
        self.buffered_event = nil
        return { consumed_bytes = 0, event = ev, error = nil }
    end

    local start_offset = self.lexer.file_offset

    while true do
        local local_start = self.lexer.file_offset - start_offset
        local slice = string.sub(buf, local_start + 1)  -- 1-based

        local token, lex_err, ok = self.lexer:read_next_token(slice, is_ending)
        local consumed = self.lexer.file_offset - start_offset

        if lex_err then
            return { consumed_bytes = consumed, event = nil, error = lex_err }
        end

        if not ok then
            -- Need more data
            break
        end

        if token then
            local ev, apply_err = self:apply_new_token(token)
            if apply_err then
                local err = self.lexer:syntax_error(
                    self.lexer.file_start_of_last_token,
                    self.lexer.file_offset,
                    apply_err
                )
                return { consumed_bytes = consumed, event = nil, error = err }
            end
            if ev then
                return { consumed_bytes = consumed, event = ev, error = nil }
            end
            -- No event but no error: loop for next token
        end
    end

    local consumed = self.lexer.file_offset - start_offset
    if is_ending then
        self.buffered_event = { kind = M.EVENT_EOF }
        local err = self.lexer:syntax_error(
            self.lexer.file_offset, self.lexer.file_offset + 1,
            "Unexpected end of file"
        )
        return { consumed_bytes = consumed, event = nil, error = err }
    end

    return { consumed_bytes = consumed, event = nil, error = nil }
end

function JSONEventGenerator:push_state_stack(state)
    if #self.state_stack > self.max_state_stack_size then
        return string.format("Max stack size of %d reached on an object opening", self.max_state_stack_size)
    end
    self.state_stack[#self.state_stack + 1] = state
    return nil  -- no error
end

-- Returns: event_table_or_nil, error_string_or_nil
function JSONEventGenerator:apply_new_token(token)
    local stack = self.state_stack
    local n = #stack

    if n == 0 then
        if self.element_read then
            if token.kind == TOKEN_EOF then
                return { kind = M.EVENT_EOF }, nil
            end
            return nil, "The JSON already contains one root element"
        end
        self.element_read = true
        return self:apply_new_token_for_value(token)
    end

    local state = stack[n]
    stack[n] = nil  -- pop

    if state == STATE_OBJECT_KEY_OR_END then
        if token.kind == TOKEN_CLOSING_CURLY then
            return { kind = M.EVENT_END_OBJECT }, nil
        end
        local err = self:push_state_stack(STATE_OBJECT_KEY)
        if err then return nil, err end
        return self:apply_new_token(token)

    elseif state == STATE_OBJECT_KEY then
        if token.kind == TOKEN_CLOSING_CURLY then
            return { kind = M.EVENT_END_OBJECT }, "Trailing commas are not allowed"
        end
        local err = self:push_state_stack(STATE_OBJECT_COLON)
        if err then return nil, err end
        if token.kind == TOKEN_STRING then
            return { kind = M.EVENT_OBJECT_KEY, value = token.value }, nil
        end
        return nil, "Object keys must be strings"

    elseif state == STATE_OBJECT_COLON then
        local err = self:push_state_stack(STATE_OBJECT_VALUE)
        if err then return nil, err end
        if token.kind == TOKEN_COLON then
            return nil, nil
        end
        local ev, _ = self:apply_new_token(token)
        if ev then self.buffered_event = ev end
        return nil, "Object keys must be followed by a colon ':'"

    elseif state == STATE_OBJECT_VALUE then
        local err = self:push_state_stack(STATE_OBJECT_COMMA_OR_END)
        if err then return nil, err end
        return self:apply_new_token_for_value(token)

    elseif state == STATE_OBJECT_COMMA_OR_END then
        if token.kind == TOKEN_COMMA then
            return nil, self:push_state_stack(STATE_OBJECT_KEY)
        elseif token.kind == TOKEN_CLOSING_CURLY then
            return { kind = M.EVENT_END_OBJECT }, nil
        else
            return nil, "Object values must be followed by a comma to add a new value or a curly bracket to end the object"
        end

    elseif state == STATE_ARRAY_VALUE_OR_END then
        if token.kind == TOKEN_CLOSING_SQUARE then
            return { kind = M.EVENT_END_ARRAY }, nil
        end
        local err = self:push_state_stack(STATE_ARRAY_VALUE)
        if err then return nil, err end
        return self:apply_new_token(token)

    elseif state == STATE_ARRAY_VALUE then
        if token.kind == TOKEN_CLOSING_SQUARE then
            return { kind = M.EVENT_END_ARRAY }, "Trailing commas are not allowed"
        end
        local err = self:push_state_stack(STATE_ARRAY_COMMA_OR_END)
        if err then return nil, err end
        return self:apply_new_token_for_value(token)

    elseif state == STATE_ARRAY_COMMA_OR_END then
        if token.kind == TOKEN_COMMA then
            return nil, self:push_state_stack(STATE_ARRAY_VALUE)
        elseif token.kind == TOKEN_CLOSING_SQUARE then
            return { kind = M.EVENT_END_ARRAY }, nil
        else
            self:push_state_stack(STATE_ARRAY_VALUE)
            local ev, _ = self:apply_new_token(token)
            if ev then self.buffered_event = ev end
            return nil, "Array values must be followed by a comma to add a new value or a squared bracket to end the array"
        end
    end

    return nil, nil
end

function JSONEventGenerator:apply_new_token_for_value(token)
    local k = token.kind
    if k == TOKEN_OPENING_SQUARE then
        return { kind = M.EVENT_START_ARRAY }, self:push_state_stack(STATE_ARRAY_VALUE_OR_END)
    elseif k == TOKEN_CLOSING_SQUARE then
        return nil, "Unexpected closing square bracket, no array to close"
    elseif k == TOKEN_OPENING_CURLY then
        return { kind = M.EVENT_START_OBJECT }, self:push_state_stack(STATE_OBJECT_KEY_OR_END)
    elseif k == TOKEN_CLOSING_CURLY then
        return nil, "Unexpected closing curly bracket, no array to close"
    elseif k == TOKEN_COMMA then
        return nil, "Unexpected comma, no values to separate"
    elseif k == TOKEN_COLON then
        return nil, "Unexpected colon, no key to follow"
    elseif k == TOKEN_STRING then
        return { kind = M.EVENT_STRING, value = token.value }, nil
    elseif k == TOKEN_NUMBER then
        return { kind = M.EVENT_NUMBER, value = token.value }, nil
    elseif k == TOKEN_TRUE then
        return { kind = M.EVENT_BOOLEAN, value = true }, nil
    elseif k == TOKEN_FALSE then
        return { kind = M.EVENT_BOOLEAN, value = false }, nil
    elseif k == TOKEN_NULL_TOK then
        return { kind = M.EVENT_NULL }, nil
    elseif k == TOKEN_EOF then
        return { kind = M.EVENT_EOF }, "Unexpected end of file, a value was expected"
    end
    return nil, nil
end

-- ── Public API ─────────────────────────────────────────────────────────────

M.JSONLexer = JSONLexer
M.JSONEventGenerator = JSONEventGenerator

return M
