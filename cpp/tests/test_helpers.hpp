#pragma once
#include "chunk_parser.hpp"
#include <cstdint>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

// ─── LCG RNG (matches Rust seed=42) ─────────────────────────────────────────

inline uint64_t lcg_next(uint64_t& state) {
    state = state * 6364136223846793005ULL + 1442695040888963407ULL;
    return state;
}

// ─── String helpers ──────────────────────────────────────────────────────────

inline std::string rep(char ch, size_t len) {
    return std::string(len, ch);
}

inline std::string text_value(char ch, size_t value_len, char quote) {
    std::string q(1, quote);
    return q + rep(ch, value_len) + q;
}

inline std::string str_array(char ch, size_t item_count, size_t value_len) {
    std::string result = "[ ";
    for (size_t i = 0; i < item_count; ++i) {
        if (i > 0) result += ",";
        result += text_value(ch, value_len, '"');
    }
    result += " ]";
    return result;
}

inline std::string obj_array(char kch, char vch, size_t item_count, size_t value_len) {
    std::string result = "[ ";
    for (size_t i = 0; i < item_count; ++i) {
        if (i > 0) result += ",";
        result += "{\"name\":" + text_value(kch, value_len, '"') +
                  ",\"value\":" + text_value(vch, value_len, '"') + "}";
    }
    result += " ]";
    return result;
}

// ─── JSON builders ───────────────────────────────────────────────────────────

inline std::string build_flat_fields(size_t field_count, size_t value_len,
                                      std::unordered_map<std::string, json>& all_expected) {
    std::string result;
    for (size_t i = 0; i < field_count; ++i) {
        char ch = 'a' + (i % 26);
        std::string key = "field_" + std::to_string(i);
        std::string value = text_value(ch, value_len, ' ');
        result += "\"" + key + "\": \"" + value + "\",";
        all_expected[key] = value;
    }
    return result;
}

inline std::vector<uint8_t> build_small_json(size_t field_count, size_t value_len,
                                              std::unordered_map<std::string, json>& all_expected) {
    std::string author    = text_value('A', value_len, ' ');
    std::string version   = text_value('B', value_len, ' ');
    std::string views     = obj_array('C', 'D', field_count, value_len);
    std::string regions   = str_array('E', field_count, value_len);
    std::string locale    = text_value('F', value_len, ' ');
    std::string name      = text_value('G', value_len, ' ');
    std::string tags      = str_array('H', field_count, value_len);
    std::string items     = obj_array('I', 'J', field_count, value_len);
    std::string key       = text_value('K', value_len, ' ');
    std::string value1    = text_value('L', value_len, ' ');
    std::string value2    = text_value('M', value_len, ' ');
    std::string signature = str_array('N', field_count, value_len);
    int timestamp = 123456789;

    // Record expected values
    all_expected["metadata."]                      = author;
    all_expected["metadata.author"]                = author;
    all_expected["metadata.version"]               = version;
    all_expected["metadata.stats.views"]           = json::parse(views, nullptr, false);
    all_expected["metadata.stats.details.regions"] = json::parse(regions, nullptr, false);
    all_expected["metadata.stats.details.locale"]  = locale;
    all_expected["metadata.name"]                  = name;
    all_expected[""]                               = json::parse(tags, nullptr, false);
    all_expected["items"]                          = json::parse(items, nullptr, false);
    all_expected["config.key"]                     = key;
    all_expected["config.values.value1"]           = value1;
    all_expected["config.values.value2"]           = value2;
    all_expected["signature"]                      = json::parse(signature, nullptr, false);
    all_expected["timestamp"]                      = timestamp;

    std::string json_str =
        "{ \"metadata\" :  {"
        "   \"\"  :  \"" + author + "\"  ,"
        "   \"author\"  :  \"" + author + "\"  ,"
        "\"version\":\"" + version + "\","
        "\"stats\":{"
          "\"views\":" + views + ","
          " \"details\" :   {  "
            "  \"regions\"   :   " + regions + ","
            "  \"locale\"   :  \"" + locale + "\"  "
          "}"
        "},"
        "\"name\": \"" + name + "\""
        "},"
        "\"\":" + tags + ","
        "\"items\":" + items + ","
        "\"config\":{"
          "\"key\": \"" + key + "\","
          "\"values\":{"
            "\"value1\": \"" + value1 + "\","
            "\"value2\": \"" + value2 + "\""
          "}"
        "},"
        "\"signature\":" + signature + ","
        "\"timestamp\":" + std::to_string(timestamp) +
        "}";

    return std::vector<uint8_t>(json_str.begin(), json_str.end());
}

inline std::vector<uint8_t> build_large_json(size_t field_count, size_t value_len,
                                              std::unordered_map<std::string, json>& all_expected) {
    std::unordered_map<std::string, json> flat_exp;
    std::string flat = build_flat_fields(field_count, value_len, flat_exp);
    for (auto& [k, v] : flat_exp) all_expected[k] = v;

    auto small = build_small_json(field_count, value_len, all_expected);
    // Strip outer braces
    std::string small_str(small.begin(), small.end());
    // remove first '{' and last '}'
    if (small_str.size() >= 2) {
        small_str = small_str.substr(1, small_str.size() - 2);
    }

    std::string json_str = "{" + flat + small_str + "}";
    return std::vector<uint8_t>(json_str.begin(), json_str.end());
}

// ─── Chunk splitting ─────────────────────────────────────────────────────────

inline std::vector<std::vector<uint8_t>>
random_chunks(const std::vector<uint8_t>& bytes,
              size_t min_size, size_t max_size,
              uint64_t seed,
              bool split_random_keys,
              const std::vector<std::string>& keys) {
    std::vector<std::vector<uint8_t>> chunks;
    size_t pos = 0;
    uint64_t rng = seed;
    bool last_split = false;

    while (pos < bytes.size()) {
        size_t range = (max_size > min_size) ? (max_size - min_size) : 1;
        size_t size  = min_size + (size_t)(lcg_next(rng) % range);
        size_t end   = std::min(pos + size, bytes.size());
        std::vector<uint8_t> chunk(bytes.begin() + pos, bytes.begin() + end);

        bool did_split = false;
        if (split_random_keys && !last_split) {
            last_split = true;
            std::string content(chunk.begin(), chunk.end());
            for (auto& key : keys) {
                if (key.empty()) continue;
                auto kpos = content.find(key);
                if (kpos != std::string::npos) {
                    size_t mid = kpos + key.size() / 2 + 1;
                    if (mid < chunk.size()) {
                        chunks.push_back(std::vector<uint8_t>(chunk.begin(), chunk.begin() + mid));
                        chunks.push_back(std::vector<uint8_t>(chunk.begin() + mid, chunk.end()));
                        did_split = true;
                        break;
                    }
                }
            }
        } else {
            last_split = false;
        }
        if (!did_split) chunks.push_back(std::move(chunk));
        pos = end;
    }
    return chunks;
}

// ─── Parser feeding ──────────────────────────────────────────────────────────

inline std::optional<size_t>
feed_chunks_to_parser(ChunkParser& parser,
                      const std::vector<std::vector<uint8_t>>& chunks,
                      bool verbose) {
    for (size_t i = 0; i < chunks.size(); ++i) {
        parser.process_chunk(chunks[i], i == chunks.size() - 1);
        if (parser.is_all_found()) return i + 1;
    }
    return std::nullopt;
}

// ─── Expected-value helpers ──────────────────────────────────────────────────

using PathMap = std::unordered_map<std::string, std::pair<std::optional<std::string>, size_t>>;

inline std::unordered_map<std::string, json>
build_expected(const PathMap& path_map,
               const std::unordered_map<std::string, json>& all_expected) {
    std::unordered_map<std::string, json> expected;
    for (auto& [json_path, val] : path_map) {
        auto it = all_expected.find(json_path);
        if (it != all_expected.end()) {
            std::string out_key = val.first ? *val.first : json_path;
            expected[out_key] = it->second;
        }
    }
    return expected;
}

inline std::vector<std::string> get_all_names(const PathMap& path_map) {
    std::vector<std::string> names;
    for (auto& [path, _] : path_map) {
        std::string part;
        for (char c : path) {
            if (c == '.') { names.push_back(part); part.clear(); }
            else part += c;
        }
        names.push_back(part);
    }
    return names;
}
