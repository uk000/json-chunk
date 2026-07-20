#pragma once
#include "parser.hpp"
// Try standard install path first, fall back to bundled single-header
#if __has_include(<nlohmann/json.hpp>)
#  include <nlohmann/json.hpp>
#else
#  include "../third_party/nlohmann/json.hpp"
#endif
#include <optional>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

using json = nlohmann::json;

// ─── ChunkEvent ───────────────────────────────────────────────────────────────

enum class ChunkEventKind {
    ObjectKey,
    String,
    Number,
    Boolean,
    Null,
    StartObject,
    EndObject,
    StartArray,
    EndArray,
    Eof,
    Ignored,
};

struct ChunkEvent {
    ChunkEventKind kind;
    std::string    string_value;
    bool           bool_value = false;
};

// ─── PathTracker ─────────────────────────────────────────────────────────────

struct PathTracker {
    std::string              path;
    std::vector<std::string> path_vector;
    std::optional<std::string> output_key;
    size_t max_value_length = 0;

    size_t                   matched_depth    = 0;
    size_t                   array_nesting    = 0;
    size_t                   skipped_depth    = 0;
    std::optional<std::string> current_key;

    bool   done             = false;
    size_t collecting_depth = 0;
    std::vector<uint8_t> collect_buffer;
    bool   overflow         = false;

    // Methods
    bool is_collecting() const { return collecting_depth > 0; }
    bool is_skipping()   const { return skipped_depth > 0; }
    bool has_data()      const { return !collect_buffer.empty(); }

    void set_current_key(const std::string& key) { current_key = key; }

    void collect(const uint8_t* b, size_t blen, bool is_new);
    void collect_start_marker(const uint8_t* b, size_t blen);
    bool is_array_of_interest();
    bool is_object_of_interest();
    bool will_collect() const;
    void move_collect_pointers(bool is_start_object, bool is_end_object,
                               bool is_start_array, bool is_end_array);
    void unwind(bool array_only);
    bool match_key(const std::string& k);
    void finish();
    void reset(bool overflow_flag);

    std::optional<json> get_value() const;
};

// ─── ChunkParser ─────────────────────────────────────────────────────────────

class ChunkParser {
public:
    // Construction
    static ChunkParser new_json_parser(
        const std::unordered_map<std::string, std::pair<std::optional<std::string>, size_t>>& path_map);

    void add_search_field(const std::string& json_path,
                          std::optional<std::string> output,
                          size_t max_size);

    void process_chunks(const std::vector<std::vector<uint8_t>>& chunks);
    void process_chunk(const std::vector<uint8_t>& chunk, bool end_of_stream);

    bool is_all_done() const;
    bool is_all_found() const;

    const PathTracker& get_field(const std::string& name) const;
    const std::unordered_map<std::string, json>& get_matches() const { return matches_found_; }
    json get_result_json();

    // Public state (mirrors Rust pub fields used in tests)
    int    json_depth    = 0;
    bool   json_started  = false;
    bool   end_of_json   = false;
    bool   end_of_stream = false;
    bool   short_circuit = false;
    bool   stop_at_first_match = true;

    std::unordered_map<std::string, PathTracker>  tracked_fields;
    std::unordered_set<std::string>               done_fields;
    std::unordered_set<std::string>               overflowed_fields;

private:
    std::vector<uint8_t>                scratch_buffer_;
    JSONEventGenerator                  json_parser_;
    std::unordered_map<std::string, json> matches_found_;

    void end_tracking();
    void end_tracker(PathTracker& tracker);
    void feed_trackers(const uint8_t* b, size_t blen);

    static ChunkParser make_empty();
};
