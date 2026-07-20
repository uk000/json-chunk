#pragma once
#include <cstdint>
#include <optional>
#include <string>
#include <variant>
#include <vector>

static constexpr size_t MAX_STATE_STACK_SIZE = 65536;

// ─── TextPosition ────────────────────────────────────────────────────────────

struct TextPosition {
    uint64_t line   = 0;
    uint64_t column = 0;
    uint64_t offset = 0;
};

// ─── JSONSyntaxError ─────────────────────────────────────────────────────────

struct JSONSyntaxError {
    TextPosition start;
    TextPosition end;
    std::string  message;

    std::string to_string() const;
};

// ─── JSONEvent ───────────────────────────────────────────────────────────────

enum class JSONEventKind {
    String,
    Number,
    Boolean,
    Null,
    StartArray,
    EndArray,
    StartObject,
    EndObject,
    ObjectKey,
    Eof,
};

struct JSONEvent {
    JSONEventKind kind;
    std::string   string_value; // for String, Number, ObjectKey
    bool          bool_value = false;  // for Boolean

    static JSONEvent make_string(std::string s)     { return {JSONEventKind::String,    std::move(s), false}; }
    static JSONEvent make_number(std::string s)     { return {JSONEventKind::Number,    std::move(s), false}; }
    static JSONEvent make_bool(bool b)              { return {JSONEventKind::Boolean,   {},            b   }; }
    static JSONEvent make_null()                    { return {JSONEventKind::Null,      {},            false}; }
    static JSONEvent make_start_array()             { return {JSONEventKind::StartArray,{},            false}; }
    static JSONEvent make_end_array()               { return {JSONEventKind::EndArray,  {},            false}; }
    static JSONEvent make_start_object()            { return {JSONEventKind::StartObject,{},           false}; }
    static JSONEvent make_end_object()              { return {JSONEventKind::EndObject, {},            false}; }
    static JSONEvent make_object_key(std::string s) { return {JSONEventKind::ObjectKey, std::move(s), false}; }
    static JSONEvent make_eof()                     { return {JSONEventKind::Eof,       {},            false}; }
};

// ─── JSONEventWrapper ────────────────────────────────────────────────────────

struct JSONEventWrapper {
    size_t consumed_bytes = 0;
    // event is absent (nullopt) when more data is needed,
    // has a JSONSyntaxError on error, or a JSONEvent on success.
    std::optional<std::variant<JSONEvent, JSONSyntaxError>> event;
};

// ─── Forward declarations ─────────────────────────────────────────────────────

class JSONLexer;

// ─── JSONEventGenerator ──────────────────────────────────────────────────────

class JSONEventGenerator {
public:
    JSONEventGenerator();

    JSONEventWrapper next_event(const uint8_t* input_buffer, size_t input_len, bool is_ending);

private:
    // Internal token types
    enum class TokenKind {
        OpeningSquareBracket,
        ClosingSquareBracket,
        OpeningCurlyBracket,
        ClosingCurlyBracket,
        Comma,
        Colon,
        String,
        Number,
        True,
        False,
        Null,
        Eof,
    };

    struct Token {
        TokenKind   kind;
        std::string value; // for String, Number
    };

    struct LexResult {
        size_t consumed = 0; // bytes consumed from buffer start
        std::optional<std::variant<Token, JSONSyntaxError>> result;
    };

    // Parser state
    enum class JSONState {
        ObjectKey,
        ObjectKeyOrEnd,
        ObjectColon,
        ObjectValue,
        ObjectCommaOrEnd,
        ArrayValue,
        ArrayValueOrEnd,
        ArrayCommaOrEnd,
    };

    // Lexer state (mirrors Rust JSONLexer)
    uint64_t file_offset_             = 0;
    uint64_t file_line_               = 0;
    uint64_t file_start_of_last_line_ = 0;
    uint64_t file_start_of_last_token_= 0;
    bool     is_start_                = true;

    std::vector<JSONState> state_stack_;
    size_t  max_state_stack_size_     = MAX_STATE_STACK_SIZE;
    bool    element_read_             = false;
    std::optional<JSONEvent> buffered_event_;

    // Lexer methods
    LexResult read_next_token(const uint8_t* buf, size_t len, bool is_ending);
    LexResult read_string(const uint8_t* buf, size_t len);
    LexResult read_constant(const uint8_t* buf, size_t len, bool is_ending,
                            const char* expected, size_t exp_len, TokenKind tk);
    LexResult read_number(const uint8_t* buf, size_t len, bool is_ending);

    JSONSyntaxError make_syntax_error(uint64_t start, uint64_t end, std::string msg);

    // Parser methods
    struct ApplyResult {
        std::optional<JSONEvent> event;
        std::optional<std::string> error;
    };
    ApplyResult apply_new_token(Token& tok);
    ApplyResult apply_new_token_for_value(Token& tok);
    std::optional<std::string> push_state_stack(JSONState s);
};
