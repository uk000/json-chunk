#include "parser.hpp"
#include <algorithm>
#include <cassert>
#include <cstring>
#include <sstream>

// ─── JSONSyntaxError::to_string ──────────────────────────────────────────────

std::string JSONSyntaxError::to_string() const {
    std::ostringstream oss;
    if (start.offset + 1 >= end.offset) {
        oss << "Parser error at line " << (start.line + 1)
            << " column " << (start.column + 1) << ": " << message;
    } else if (start.line == end.line) {
        oss << "Parser error at line " << (start.line + 1)
            << " between columns " << (start.column + 1)
            << " and column " << (end.column + 1) << ": " << message;
    } else {
        oss << "Parser error between line " << (start.line + 1)
            << " column " << (start.column + 1)
            << " and line " << (end.line + 1)
            << " column " << (end.column + 1) << ": " << message;
    }
    return oss.str();
}

// ─── helpers ─────────────────────────────────────────────────────────────────

static bool is_hex_digit(uint8_t c) {
    return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
}

static uint32_t hex_val(uint8_t c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    return c - 'A' + 10;
}

// Read exactly 4 hex chars from buf[0..4]; return code point or error.
static std::variant<uint32_t, std::string> read_hexa_char(const uint8_t* buf) {
    uint32_t val = 0;
    for (int i = 0; i < 4; ++i) {
        uint8_t c = buf[i];
        if (!is_hex_digit(c)) {
            std::string err = "Unexpected character in a unicode escape: '";
            err += (char)c;
            err += "'";
            return err;
        }
        val = val * 16 + hex_val(c);
    }
    return val;
}

// Encode a Unicode code point to UTF-8; appends to `out`.
static void encode_utf8(uint32_t cp, std::string& out) {
    if (cp < 0x80) {
        out += (char)cp;
    } else if (cp < 0x800) {
        out += (char)(0xC0 | (cp >> 6));
        out += (char)(0x80 | (cp & 0x3F));
    } else if (cp < 0x10000) {
        out += (char)(0xE0 | (cp >> 12));
        out += (char)(0x80 | ((cp >> 6) & 0x3F));
        out += (char)(0x80 | (cp & 0x3F));
    } else {
        out += (char)(0xF0 | (cp >> 18));
        out += (char)(0x80 | ((cp >> 12) & 0x3F));
        out += (char)(0x80 | ((cp >> 6) & 0x3F));
        out += (char)(0x80 | (cp & 0x3F));
    }
}

// Validate UTF-8 and append to out. Returns error position offset if invalid.
// Returns -1 if all OK.
static int validate_utf8(const uint8_t* buf, size_t len, std::string& out) {
    size_t i = 0;
    while (i < len) {
        uint8_t b = buf[i];
        int seqlen;
        if (b < 0x80) { seqlen = 1; }
        else if ((b & 0xE0) == 0xC0) { seqlen = 2; }
        else if ((b & 0xF0) == 0xE0) { seqlen = 3; }
        else if ((b & 0xF8) == 0xF0) { seqlen = 4; }
        else { out += '\xEF'; out += '\xBF'; out += '\xBD'; ++i; return (int)i - 1; }
        if (i + seqlen > len) {
            out += '\xEF'; out += '\xBF'; out += '\xBD';
            return (int)i;
        }
        for (int j = 1; j < seqlen; ++j) {
            if ((buf[i + j] & 0xC0) != 0x80) {
                out += '\xEF'; out += '\xBF'; out += '\xBD';
                return (int)i;
            }
        }
        for (int j = 0; j < seqlen; ++j) out += (char)buf[i + j];
        i += seqlen;
    }
    return -1;
}

// Count digits starting at buf, up to len bytes. Returns nullopt if all are digits and !is_ending (need more).
static std::optional<size_t> read_digits(const uint8_t* buf, size_t len, bool is_ending) {
    size_t count = 0;
    while (count < len && buf[count] >= '0' && buf[count] <= '9') ++count;
    if (count == len && !is_ending) return std::nullopt;
    return count;
}

// ─── JSONEventGenerator ──────────────────────────────────────────────────────

JSONEventGenerator::JSONEventGenerator() = default;

JSONSyntaxError JSONEventGenerator::make_syntax_error(uint64_t s, uint64_t e, std::string msg) {
    uint64_t adj_s = std::max(s, file_start_of_last_line_);
    TextPosition sp, ep;
    sp.line   = file_line_;
    sp.column = adj_s - file_start_of_last_line_;
    sp.offset = adj_s;
    ep.line   = file_line_;
    ep.column = e - file_start_of_last_line_;
    ep.offset = e;
    return JSONSyntaxError{sp, ep, std::move(msg)};
}

std::optional<std::string> JSONEventGenerator::push_state_stack(JSONState s) {
    if (state_stack_.size() > max_state_stack_size_) {
        return std::string("Max stack size of ") + std::to_string(max_state_stack_size_) +
               " reached on an object opening";
    }
    state_stack_.push_back(s);
    return std::nullopt;
}

JSONEventGenerator::ApplyResult JSONEventGenerator::apply_new_token_for_value(Token& tok) {
    switch (tok.kind) {
    case TokenKind::OpeningSquareBracket: {
        auto err = push_state_stack(JSONState::ArrayValueOrEnd);
        return {JSONEvent::make_start_array(), err};
    }
    case TokenKind::ClosingSquareBracket:
        return {std::nullopt, std::string("Unexpected closing square bracket, no array to close")};
    case TokenKind::OpeningCurlyBracket: {
        auto err = push_state_stack(JSONState::ObjectKeyOrEnd);
        return {JSONEvent::make_start_object(), err};
    }
    case TokenKind::ClosingCurlyBracket:
        return {std::nullopt, std::string("Unexpected closing curly bracket, no array to close")};
    case TokenKind::Comma:
        return {std::nullopt, std::string("Unexpected comma, no values to separate")};
    case TokenKind::Colon:
        return {std::nullopt, std::string("Unexpected colon, no key to follow")};
    case TokenKind::String:
        return {JSONEvent::make_string(std::move(tok.value)), std::nullopt};
    case TokenKind::Number:
        return {JSONEvent::make_number(std::move(tok.value)), std::nullopt};
    case TokenKind::True:
        return {JSONEvent::make_bool(true), std::nullopt};
    case TokenKind::False:
        return {JSONEvent::make_bool(false), std::nullopt};
    case TokenKind::Null:
        return {JSONEvent::make_null(), std::nullopt};
    case TokenKind::Eof:
        return {JSONEvent::make_eof(), std::string("Unexpected end of file, a value was expected")};
    }
    return {};
}

JSONEventGenerator::ApplyResult JSONEventGenerator::apply_new_token(Token& tok) {
    if (state_stack_.empty()) {
        if (element_read_) {
            if (tok.kind == TokenKind::Eof) {
                return {JSONEvent::make_eof(), std::nullopt};
            }
            return {std::nullopt, std::string("The JSON already contains one root element")};
        }
        element_read_ = true;
        return apply_new_token_for_value(tok);
    }

    JSONState top = state_stack_.back();
    state_stack_.pop_back();

    switch (top) {
    case JSONState::ObjectKeyOrEnd:
        if (tok.kind == TokenKind::ClosingCurlyBracket) {
            return {JSONEvent::make_end_object(), std::nullopt};
        } else {
            auto err = push_state_stack(JSONState::ObjectKey);
            if (err) return {std::nullopt, err};
            return apply_new_token(tok);
        }

    case JSONState::ObjectKey:
        if (tok.kind == TokenKind::ClosingCurlyBracket) {
            return {JSONEvent::make_end_object(), std::string("Trailing commas are not allowed")};
        }
        {
            auto err = push_state_stack(JSONState::ObjectColon);
            if (err) return {std::nullopt, err};
            if (tok.kind == TokenKind::String) {
                return {JSONEvent::make_object_key(std::move(tok.value)), std::nullopt};
            }
            return {std::nullopt, std::string("Object keys must be strings")};
        }

    case JSONState::ObjectColon: {
        auto err = push_state_stack(JSONState::ObjectValue);
        if (err) return {std::nullopt, err};
        if (tok.kind == TokenKind::Colon) {
            return {std::nullopt, std::nullopt};
        }
        auto res = apply_new_token(tok);
        res.error = std::string("Object keys must be followed by a colon ':'");
        return res;
    }

    case JSONState::ObjectValue: {
        auto err = push_state_stack(JSONState::ObjectCommaOrEnd);
        if (err) return {std::nullopt, err};
        return apply_new_token_for_value(tok);
    }

    case JSONState::ObjectCommaOrEnd:
        if (tok.kind == TokenKind::Comma) {
            auto err = push_state_stack(JSONState::ObjectKey);
            return {std::nullopt, err};
        }
        if (tok.kind == TokenKind::ClosingCurlyBracket) {
            return {JSONEvent::make_end_object(), std::nullopt};
        }
        return {std::nullopt,
            std::string("Object values must be followed by a comma to add a new value or a curly bracket to end the object")};

    case JSONState::ArrayValueOrEnd:
        if (tok.kind == TokenKind::ClosingSquareBracket) {
            return {JSONEvent::make_end_array(), std::nullopt};
        } else {
            auto err = push_state_stack(JSONState::ArrayValue);
            if (err) return {std::nullopt, err};
            return apply_new_token(tok);
        }

    case JSONState::ArrayValue:
        if (tok.kind == TokenKind::ClosingSquareBracket) {
            return {JSONEvent::make_end_array(), std::string("Trailing commas are not allowed")};
        } else {
            auto err = push_state_stack(JSONState::ArrayCommaOrEnd);
            if (err) return {std::nullopt, err};
            return apply_new_token_for_value(tok);
        }

    case JSONState::ArrayCommaOrEnd:
        if (tok.kind == TokenKind::Comma) {
            auto err = push_state_stack(JSONState::ArrayValue);
            return {std::nullopt, err};
        }
        if (tok.kind == TokenKind::ClosingSquareBracket) {
            return {JSONEvent::make_end_array(), std::nullopt};
        }
        // error recovery: push ArrayValue back, continue
        push_state_stack(JSONState::ArrayValue);
        {
            auto res = apply_new_token(tok);
            res.error = std::string("Array values must be followed by a comma to add a new value or a squared bracket to end the array");
            return res;
        }
    }
    return {};
}

JSONEventWrapper JSONEventGenerator::next_event(const uint8_t* input_buffer, size_t input_len,
                                                 bool is_ending) {
    if (buffered_event_) {
        JSONEvent ev = std::move(*buffered_event_);
        buffered_event_.reset();
        return JSONEventWrapper{0, std::variant<JSONEvent, JSONSyntaxError>(std::move(ev))};
    }

    uint64_t start_file_offset = file_offset_;

    while (true) {
        size_t offset_in_buf = (size_t)(file_offset_ - start_file_offset);
        const uint8_t* slice = input_buffer + offset_in_buf;
        size_t slice_len     = (offset_in_buf <= input_len) ? (input_len - offset_in_buf) : 0;

        LexResult lr = read_next_token(slice, slice_len, is_ending);

        size_t consumed = (size_t)(file_offset_ - start_file_offset);

        if (!lr.result.has_value()) {
            // Need more data
            if (is_ending) {
                // Unexpected EOF
                buffered_event_ = JSONEvent::make_eof();
                auto err = make_syntax_error(file_offset_, file_offset_ + 1, "Unexpected end of file");
                return JSONEventWrapper{consumed, std::variant<JSONEvent, JSONSyntaxError>(std::move(err))};
            }
            return JSONEventWrapper{consumed, std::nullopt};
        }

        if (std::holds_alternative<JSONSyntaxError>(*lr.result)) {
            return JSONEventWrapper{consumed,
                std::variant<JSONEvent, JSONSyntaxError>(std::move(std::get<JSONSyntaxError>(*lr.result)))};
        }

        Token& tok = std::get<Token>(*lr.result);
        ApplyResult ar = apply_new_token(tok);

        if (ar.error) {
            auto err = make_syntax_error(file_start_of_last_token_, file_offset_, *ar.error);
            if (ar.event) {
                buffered_event_ = std::move(*ar.event);
            }
            return JSONEventWrapper{consumed, std::variant<JSONEvent, JSONSyntaxError>(std::move(err))};
        }

        if (ar.event) {
            return JSONEventWrapper{consumed,
                std::variant<JSONEvent, JSONSyntaxError>(std::move(*ar.event))};
        }
        // No event yet — keep reading tokens
    }
}

// ─── Lexer ────────────────────────────────────────────────────────────────────

JSONEventGenerator::LexResult JSONEventGenerator::read_next_token(const uint8_t* buf, size_t len,
                                                                    bool is_ending) {
    // BOM handling
    if (is_start_) {
        if (len < 3 && !is_ending) return LexResult{};
        is_start_ = false;
        if (len >= 3 && buf[0] == 0xEF && buf[1] == 0xBB && buf[2] == 0xBF) {
            buf += 3; len -= 3;
            file_offset_ += 3;
        }
    }

    // Skip whitespace
    size_t i = 0;
    while (i < len) {
        uint8_t c = buf[i];
        if (c == ' ' || c == '\t') {
            ++i;
        } else if (c == '\n') {
            ++i;
            ++file_line_;
            file_start_of_last_line_ = file_offset_ + i;
        } else if (c == '\r') {
            ++i;
            if (i < len) {
                if (buf[i] == '\n') ++i;
            } else if (!is_ending) {
                --i; // need more to know if \r\n
                file_offset_ += i;
                return LexResult{};
            }
            ++file_line_;
            file_start_of_last_line_ = file_offset_ + i;
        } else {
            break;
        }
    }
    file_offset_ += i;
    buf += i; len -= i;
    file_start_of_last_token_ = file_offset_;

    if (is_ending && len == 0) {
        Token t; t.kind = TokenKind::Eof;
        return LexResult{0, std::variant<Token, JSONSyntaxError>(std::move(t))};
    }
    if (len == 0) return LexResult{};

    uint8_t c = buf[0];
    switch (c) {
    case '{': file_offset_ += 1; { Token t; t.kind = TokenKind::OpeningCurlyBracket;  return LexResult{0, std::variant<Token,JSONSyntaxError>(t)}; }
    case '}': file_offset_ += 1; { Token t; t.kind = TokenKind::ClosingCurlyBracket;  return LexResult{0, std::variant<Token,JSONSyntaxError>(t)}; }
    case '[': file_offset_ += 1; { Token t; t.kind = TokenKind::OpeningSquareBracket; return LexResult{0, std::variant<Token,JSONSyntaxError>(t)}; }
    case ']': file_offset_ += 1; { Token t; t.kind = TokenKind::ClosingSquareBracket; return LexResult{0, std::variant<Token,JSONSyntaxError>(t)}; }
    case ',': file_offset_ += 1; { Token t; t.kind = TokenKind::Comma;                return LexResult{0, std::variant<Token,JSONSyntaxError>(t)}; }
    case ':': file_offset_ += 1; { Token t; t.kind = TokenKind::Colon;                return LexResult{0, std::variant<Token,JSONSyntaxError>(t)}; }
    case '"': return read_string(buf, len);
    case 't': return read_constant(buf, len, is_ending, "true",  4, TokenKind::True);
    case 'f': return read_constant(buf, len, is_ending, "false", 5, TokenKind::False);
    case 'n': return read_constant(buf, len, is_ending, "null",  4, TokenKind::Null);
    default:
        if (c == '-' || (c >= '0' && c <= '9')) return read_number(buf, len, is_ending);
        file_offset_ += 1;
        std::string msg;
        if (c < 128) { msg = "Unexpected char: '"; msg += (char)c; msg += "'"; }
        else { msg = "Unexpected byte: \\x"; char hex[3]; snprintf(hex,3,"%02X",c); msg += hex; }
        auto err = make_syntax_error(file_offset_ - 1, file_offset_, std::move(msg));
        return LexResult{0, std::variant<Token, JSONSyntaxError>(std::move(err))};
    }
}

JSONEventGenerator::LexResult JSONEventGenerator::read_string(const uint8_t* buf, size_t len) {
    // buf[0] == '"'
    std::optional<JSONSyntaxError> error;
    // When we encounter an escape sequence, we need to build a heap string.
    // built_string accumulates the decoded value; read_until tracks up to
    // where we've already pushed bytes into built_string (relative to buf+1).
    bool has_built = false;
    std::string built_string;
    size_t read_until = 1; // start after the opening quote

    size_t i = 1;
    while (true) {
        if (i >= len) return LexResult{}; // need more data

        uint8_t c = buf[i];
        if (c == '"') {
            // End of string
            LexResult res;
            if (error) {
                file_offset_ += i + 1;
                res.result = std::variant<Token, JSONSyntaxError>(std::move(*error));
                return res;
            }
            if (has_built) {
                // flush remaining bytes since read_until
                if (read_until < i) {
                    int invalid = validate_utf8(buf + read_until, i - read_until, built_string);
                    if (invalid >= 0 && !error) {
                        uint64_t pos = file_offset_ + read_until + invalid;
                        error = make_syntax_error(pos, pos + 1, "Invalid UTF-8");
                    }
                }
                file_offset_ += i + 1;
                if (error) {
                    res.result = std::variant<Token, JSONSyntaxError>(std::move(*error));
                } else {
                    Token t; t.kind = TokenKind::String; t.value = std::move(built_string);
                    res.result = std::variant<Token, JSONSyntaxError>(std::move(t));
                }
            } else {
                // No escapes — validate + borrow
                std::string s;
                int invalid = validate_utf8(buf + 1, i - 1, s);
                if (invalid >= 0 && !error) {
                    uint64_t pos = file_offset_ + 1 + invalid;
                    error = make_syntax_error(pos, pos + 1, "Invalid UTF-8");
                }
                file_offset_ += i + 1;
                if (error) {
                    res.result = std::variant<Token, JSONSyntaxError>(std::move(*error));
                } else {
                    Token t; t.kind = TokenKind::String; t.value = std::move(s);
                    res.result = std::variant<Token, JSONSyntaxError>(std::move(t));
                }
            }
            return res;
        } else if (c == '\\') {
            if (!has_built) {
                has_built = true;
                built_string.clear();
                read_until = 1;
            }
            // flush from read_until to i into built_string
            if (read_until < i) {
                int invalid = validate_utf8(buf + read_until, i - read_until, built_string);
                if (invalid >= 0 && !error) {
                    uint64_t pos = file_offset_ + read_until + invalid;
                    error = make_syntax_error(pos, pos + 1, "Invalid UTF-8");
                }
            }
            ++i;
            if (i >= len) return LexResult{};
            uint8_t esc = buf[i];
            switch (esc) {
            case '"':  built_string += '"';  ++i; break;
            case '\\': built_string += '\\'; ++i; break;
            case '/':  built_string += '/';  ++i; break;
            case 'b':  built_string += '\x08'; ++i; break;
            case 'f':  built_string += '\x0C'; ++i; break;
            case 'n':  built_string += '\n'; ++i; break;
            case 'r':  built_string += '\r'; ++i; break;
            case 't':  built_string += '\t'; ++i; break;
            case 'u': {
                ++i;
                if (i + 4 > len) return LexResult{};
                auto cp_or_err = read_hexa_char(buf + i);
                uint32_t code_point = 0;
                if (std::holds_alternative<std::string>(cp_or_err)) {
                    if (!error) {
                        uint64_t pos = file_offset_ + i + 4;
                        error = make_syntax_error(pos - 4, pos, std::get<std::string>(cp_or_err));
                    }
                    built_string += '\xEF'; built_string += '\xBF'; built_string += '\xBD'; // replacement char
                    i += 4;
                } else {
                    code_point = std::get<uint32_t>(cp_or_err);
                    i += 4;
                    // Check if it's a surrogate
                    if (code_point >= 0xD800 && code_point <= 0xDFFF) {
                        uint32_t high_surrogate = code_point;
                        if (!(high_surrogate >= 0xD800 && high_surrogate <= 0xDBFF)) {
                            if (!error) {
                                uint64_t pos = file_offset_ + i;
                                error = make_syntax_error(pos - 6, pos,
                                    "\\u" + std::to_string(high_surrogate) + " is not a valid high surrogate");
                            }
                        }
                        // Need 6 more bytes: \uXXXX
                        if (i + 6 > len) return LexResult{};
                        if (buf[i] != '\\' || buf[i+1] != 'u') {
                            if (!error) {
                                uint64_t pos = file_offset_ + i + 6;
                                error = make_syntax_error(pos - 6, pos,
                                    std::string("\\u") + std::to_string(high_surrogate) +
                                    " is a high surrogate and should be followed by a low surrogate \\uXXXX");
                            }
                        }
                        auto ls_or_err = read_hexa_char(buf + i + 2);
                        uint32_t low_surrogate = 0;
                        if (std::holds_alternative<std::string>(ls_or_err)) {
                            if (!error) {
                                uint64_t pos = file_offset_ + i + 6;
                                error = make_syntax_error(pos - 4, pos, std::get<std::string>(ls_or_err));
                            }
                            low_surrogate = 0xDC00; // fake valid low to continue
                        } else {
                            low_surrogate = std::get<uint32_t>(ls_or_err);
                        }
                        i += 6;
                        if (!(low_surrogate >= 0xDC00 && low_surrogate <= 0xDFFF)) {
                            if (!error) {
                                uint64_t pos = file_offset_ + i;
                                error = make_syntax_error(pos - 6, pos,
                                    std::string("\\u") + std::to_string(low_surrogate) + " is not a valid low surrogate");
                            }
                        }
                        uint32_t cp = 0x10000 + ((high_surrogate & 0x03FF) << 10) + (low_surrogate & 0x03FF);
                        encode_utf8(cp, built_string);
                    } else {
                        encode_utf8(code_point, built_string);
                    }
                }
                break;
            }
            default: {
                ++i;
                if (!error) {
                    uint64_t pos = file_offset_ + i;
                    std::string msg = "'\\";
                    if (esc < 128) msg += (char)esc;
                    msg += "' is not a valid escape sequence";
                    error = make_syntax_error(pos - 2, pos, std::move(msg));
                }
                built_string += '\xEF'; built_string += '\xBF'; built_string += '\xBD';
                break;
            }
            }
            read_until = i;
        } else if (c <= 0x1F) {
            if (!error) {
                uint64_t pos = file_offset_ + i;
                std::string msg = "'";
                msg += (char)c;
                msg += "' is not allowed in JSON strings";
                error = make_syntax_error(pos, pos + 1, std::move(msg));
            }
            ++i;
        } else {
            ++i;
        }
    }
}

JSONEventGenerator::LexResult JSONEventGenerator::read_constant(
    const uint8_t* buf, size_t len, bool is_ending,
    const char* expected, size_t exp_len, TokenKind tk) {
    if (len >= exp_len && memcmp(buf, expected, exp_len) == 0) {
        file_offset_ += exp_len;
        Token t; t.kind = tk;
        return LexResult{0, std::variant<Token, JSONSyntaxError>(std::move(t))};
    }
    // Count alphabetic prefix
    size_t ascii_chars = 0;
    while (ascii_chars < len && ((buf[ascii_chars] >= 'a' && buf[ascii_chars] <= 'z') ||
                                  (buf[ascii_chars] >= 'A' && buf[ascii_chars] <= 'Z'))) {
        ++ascii_chars;
    }
    if (ascii_chars == len && !is_ending) return LexResult{};
    size_t read = std::max((size_t)1, ascii_chars);
    uint64_t start = file_offset_;
    file_offset_ += read;
    std::string msg = std::string(expected) + " expected";
    auto err = make_syntax_error(start, file_offset_, std::move(msg));
    return LexResult{0, std::variant<Token, JSONSyntaxError>(std::move(err))};
}

JSONEventGenerator::LexResult JSONEventGenerator::read_number(const uint8_t* buf, size_t len,
                                                               bool is_ending) {
    size_t i = 0;
    if (i < len && buf[i] == '-') ++i;

    if (i >= len) return LexResult{};
    uint8_t c = buf[i];

    if (c == '0') {
        ++i;
    } else if (c >= '1' && c <= '9') {
        ++i;
        auto cnt = read_digits(buf + i, len - i, is_ending);
        if (!cnt) return LexResult{};
        i += *cnt;
    } else {
        ++i;
        file_offset_ += i;
        std::string msg = "A number is not allowed to start with '";
        msg += (char)c; msg += "'";
        auto err = make_syntax_error(file_offset_ - 1, file_offset_, std::move(msg));
        return LexResult{0, std::variant<Token, JSONSyntaxError>(std::move(err))};
    }

    // Dot
    {
        std::optional<uint8_t> next;
        if (i < len) next = buf[i];
        else if (is_ending) next = std::nullopt; // no dot
        else return LexResult{};

        if (next == (uint8_t)'.') {
            ++i;
            if (i >= len) return LexResult{};
            uint8_t fc = buf[i]; ++i;
            if (!(fc >= '0' && fc <= '9')) {
                file_offset_ += i;
                std::string msg = "A number fractional part must start with a digit and not '";
                msg += (char)fc; msg += "'";
                auto err = make_syntax_error(file_offset_ - 1, file_offset_, std::move(msg));
                return LexResult{0, std::variant<Token, JSONSyntaxError>(std::move(err))};
            }
            auto cnt = read_digits(buf + i, len - i, is_ending);
            if (!cnt) return LexResult{};
            i += *cnt;
        }
    }

    // Exponent
    {
        std::optional<uint8_t> next;
        if (i < len) next = buf[i];
        else if (is_ending) next = std::nullopt;
        else return LexResult{};

        if (next == (uint8_t)'e' || next == (uint8_t)'E') {
            ++i;
            if (i >= len) return LexResult{};
            uint8_t ec = buf[i];
            if (ec == '-' || ec == '+') {
                ++i;
                if (i >= len) return LexResult{};
                uint8_t dc = buf[i]; ++i;
                if (!(dc >= '0' && dc <= '9')) {
                    file_offset_ += i;
                    std::string msg = "A number exponential part must contain at least a digit, '";
                    msg += (char)dc; msg += "' found";
                    auto err = make_syntax_error(file_offset_ - 1, file_offset_, std::move(msg));
                    return LexResult{0, std::variant<Token, JSONSyntaxError>(std::move(err))};
                }
            } else if (ec >= '0' && ec <= '9') {
                ++i;
            } else {
                ++i;
                file_offset_ += i;
                std::string msg = "A number exponential part must start with +, - or a digit, '";
                msg += (char)ec; msg += "' found";
                auto err = make_syntax_error(file_offset_ - 1, file_offset_, std::move(msg));
                return LexResult{0, std::variant<Token, JSONSyntaxError>(std::move(err))};
            }
            auto cnt = read_digits(buf + i, len - i, is_ending);
            if (!cnt) return LexResult{};
            i += *cnt;
        }
    }

    file_offset_ += i;
    Token t;
    t.kind  = TokenKind::Number;
    t.value = std::string(reinterpret_cast<const char*>(buf), i);
    return LexResult{0, std::variant<Token, JSONSyntaxError>(std::move(t))};
}
