#include "chunk_parser.hpp"
#include "test_helpers.hpp"
#include <cassert>
#include <cstdio>
#include <iostream>
#include <string>
#include <unordered_map>

// ─── Minimal test framework ───────────────────────────────────────────────────

static int g_tests_run = 0;
static int g_tests_failed = 0;

#define ASSERT_EQ(a, b) do { \
    auto _a = (a); auto _b = (b); \
    if (!(_a == _b)) { \
        std::cerr << "  FAIL: " << #a << " != " << #b \
                  << " (" << _a << " vs " << _b << ") at " << __FILE__ << ":" << __LINE__ << "\n"; \
        ++g_tests_failed; \
    } \
} while(0)

#define ASSERT_TRUE(x) do { \
    if (!(x)) { \
        std::cerr << "  FAIL: " << #x << " is false at " << __FILE__ << ":" << __LINE__ << "\n"; \
        ++g_tests_failed; \
    } \
} while(0)

#define ASSERT_FALSE(x) ASSERT_TRUE(!(x))

#define RUN_TEST(name) do { \
    ++g_tests_run; \
    std::cout << "[ RUN  ] " << #name << "\n"; \
    int prev_failed = g_tests_failed; \
    name(); \
    if (g_tests_failed == prev_failed) std::cout << "[  OK  ] " << #name << "\n"; \
    else std::cout << "[ FAIL ] " << #name << "\n"; \
} while(0)

// ─── Tests ────────────────────────────────────────────────────────────────────

void test_split_numeric_field() {
    std::string json_str = "{\"timestamp\":123456789}";
    std::vector<uint8_t> bytes(json_str.begin(), json_str.end());

    PathMap path_map = {
        {"timestamp", {std::nullopt, 100}},
    };

    std::vector<std::string> names = {"timestamp"};
    auto chunks = random_chunks(bytes, 10, 50, 42, true, names);

    ChunkParser parser = ChunkParser::new_json_parser(path_map);
    auto last = feed_chunks_to_parser(parser, chunks, false);

    ASSERT_TRUE(parser.is_all_found());
    ASSERT_TRUE(last.has_value());

    auto& matches = parser.get_matches();
    ASSERT_TRUE(matches.count("timestamp") > 0);
    ASSERT_EQ(matches.at("timestamp").get<int>(), 123456789);
    std::cout << "  timestamp = " << matches.at("timestamp") << "\n";
}

void test_flat_fields_basic() {
    std::string json_str = R"({"method":"tools/call","params":{"name":"myTool","args":{"x":1}}})";
    std::vector<uint8_t> bytes(json_str.begin(), json_str.end());

    PathMap path_map = {
        {"method",      {std::nullopt, 100}},
        {"params.name", {std::nullopt, 100}},
    };

    std::vector<std::string> names = {"method", "params", "name"};
    auto chunks = random_chunks(bytes, 5, 20, 42, true, names);

    ChunkParser parser = ChunkParser::new_json_parser(path_map);
    auto last = feed_chunks_to_parser(parser, chunks, false);

    ASSERT_TRUE(parser.is_all_found());
    auto& m = parser.get_matches();
    ASSERT_TRUE(m.count("method") > 0);
    ASSERT_TRUE(m.count("params.name") > 0);
    ASSERT_EQ(m.at("method").get<std::string>(), "tools/call");
    ASSERT_EQ(m.at("params.name").get<std::string>(), "myTool");
    std::cout << "  method=" << m.at("method") << " params.name=" << m.at("params.name") << "\n";
}

void test_mix_flat_nested_fields() {
    std::unordered_map<std::string, json> all_expected;
    auto bytes = build_large_json(10, 50, all_expected);

    PathMap path_map = {
        {"field_1",                       {std::string("b"),      0}},
        {"metadata.stats.details.locale", {std::string("locale"), 100}},
        {"config.key",                    {std::nullopt,           100}},
        {"config.values.value2",          {std::string("value"),  0}},
        {"timestamp",                     {std::nullopt,           10}},
    };

    auto expected = build_expected(path_map, all_expected);
    auto names    = get_all_names(path_map);
    auto chunks   = random_chunks(bytes, 10, 50, 42, true, names);

    ChunkParser parser = ChunkParser::new_json_parser(path_map);
    auto last = feed_chunks_to_parser(parser, chunks, false);

    ASSERT_TRUE(parser.is_all_found());
    auto& m = parser.get_matches();
    for (auto& [k, v] : expected) {
        ASSERT_TRUE(m.count(k) > 0);
        if (m.count(k)) {
            bool eq = (m.at(k) == v);
            if (!eq) {
                std::cerr << "  MISMATCH key=" << k << " expected=" << v << " got=" << m.at(k) << "\n";
                ++g_tests_failed;
            }
        }
    }
}

void test_overflow() {
    std::unordered_map<std::string, json> all_expected;
    auto bytes = build_large_json(3, 1000, all_expected);

    PathMap path_map = {
        {"field_1",                       {std::string("b"),      0}},   // max_size=0 → no overflow cap
        {"metadata.stats.details.locale", {std::string("locale"), 200}}, // will overflow (locale ~1000 chars)
        {"config.key",                    {std::nullopt,           100}}, // will overflow
        {"config.values.value2",          {std::string("value"),  0}},   // no cap
    };

    auto names  = get_all_names(path_map);
    auto chunks = random_chunks(bytes, 50, 300, 42, true, names);

    ChunkParser parser = ChunkParser::new_json_parser(path_map);
    feed_chunks_to_parser(parser, chunks, false);

    ASSERT_TRUE(parser.is_all_found());
    ASSERT_EQ(parser.get_field("field_1").overflow,                       false);
    ASSERT_EQ(parser.get_field("metadata.stats.details.locale").overflow, true);
    ASSERT_EQ(parser.get_field("config.key").overflow,                    true);
    ASSERT_EQ(parser.get_field("config.values.value2").overflow,          false);
}

void test_invalid_json_fields() {
    std::unordered_map<std::string, json> all_expected;
    auto bytes = build_large_json(20, 2000, all_expected);

    PathMap path_map = {
        {"field_x",               {std::nullopt,             0}},
        {"metadata.foo.details.region", {std::string("region"), 512}},
        {"foo.name",              {std::nullopt,             256}},
    };

    auto chunks = random_chunks(bytes, 50, 300, 42, false, {});

    ChunkParser parser = ChunkParser::new_json_parser(path_map);
    feed_chunks_to_parser(parser, chunks, true);

    // None of the paths exist → nothing found
    ASSERT_FALSE(parser.is_all_found());
    ASSERT_EQ((int)parser.get_matches().size(), 0);
}

void test_multi_nested_obj_arrays() {
    std::unordered_map<std::string, json> all_expected;
    auto bytes = build_large_json(2, 10, all_expected);

    PathMap path_map = {
        {"metadata.stats.details.regions", {std::string("regions"), 0}},
        {"config.values",                  {std::string("values"),  0}},
        {"metadata.name",                  {std::nullopt,            0}},
    };

    auto expected = build_expected(path_map, all_expected);
    auto names    = get_all_names(path_map);
    auto chunks   = random_chunks(bytes, 10, 50, 42, true, names);

    ChunkParser parser = ChunkParser::new_json_parser(path_map);
    auto last = feed_chunks_to_parser(parser, chunks, false);

    ASSERT_TRUE(parser.is_all_found());
    ASSERT_TRUE(last.has_value());

    // Verify collected array/object values match expected (tests byte-accounting across chunks)
    auto& m = parser.get_matches();
    for (auto& [k, v] : expected) {
        ASSERT_TRUE(m.count(k) > 0);
        if (m.count(k)) {
            bool eq = (m.at(k) == v);
            if (!eq) {
                std::cerr << "  MISMATCH key=" << k
                          << "\n  expected=" << v.dump()
                          << "\n  got="      << m.at(k).dump() << "\n";
                ++g_tests_failed;
            }
        }
    }
}

void test_short_circuit() {
    std::unordered_map<std::string, json> all_expected;
    auto bytes = build_large_json(10, 10, all_expected);

    // Single field — short-circuit should kick in
    PathMap path_map = {{"field_0", {std::nullopt, 100}}};
    auto names  = get_all_names(path_map);
    auto chunks = random_chunks(bytes, 10, 50, 42, true, names);

    ChunkParser parser = ChunkParser::new_json_parser(path_map);
    auto last = feed_chunks_to_parser(parser, chunks, false);

    ASSERT_TRUE(parser.is_all_found());
    ASSERT_TRUE(last.has_value());
    // short_circuit should have fired before reaching the end
    std::cout << "  Exited at chunk " << *last << " / " << chunks.size() << "\n";
}

void test_object_with_empty_key_path() {
    std::unordered_map<std::string, json> all_expected;
    auto bytes = build_small_json(3, 10, all_expected);

    PathMap path_map = {
        {"metadata.",   {std::string("metadata"), 100}},
        {"config.key",  {std::nullopt,             100}},
    };

    auto names  = get_all_names(path_map);
    auto chunks = random_chunks(bytes, 10, 50, 42, true, names);

    ChunkParser parser = ChunkParser::new_json_parser(path_map);
    feed_chunks_to_parser(parser, chunks, false);

    ASSERT_TRUE(parser.is_all_found());
    auto& m = parser.get_matches();
    ASSERT_TRUE(m.count("metadata") > 0);
    ASSERT_TRUE(m.count("config.key") > 0);
}

void test_detect_json_end_no_eos() {
    // Mirrors Rust: JSON followed by garbage, searching for a key that doesn't exist.
    // is_ending is always false — parser must detect end-of-JSON by depth returning to 0.
    std::unordered_map<std::string, json> all_expected;
    auto valid = build_small_json(3, 10, all_expected);
    std::string garbage = "-------+++++++";
    valid.insert(valid.end(), garbage.begin(), garbage.end());

    PathMap path_map = {
        {"config.key",           {std::nullopt,          100}},
        {"config.values.xxx",    {std::string("value"),  0}},  // does not exist
    };

    auto names  = get_all_names(path_map);
    auto chunks = random_chunks(valid, 50, 300, 42, true, names);

    ChunkParser parser = ChunkParser::new_json_parser(path_map);
    for (size_t i = 0; i < chunks.size(); ++i) {
        parser.process_chunk(chunks[i], false); // never tell parser stream ended
        if (parser.is_all_found()) break;
    }

    std::cout << "  json_depth=" << parser.json_depth
              << " end_of_json=" << parser.end_of_json
              << " short_circuit=" << parser.short_circuit << "\n";

    // config.values.xxx does not exist → not all found
    ASSERT_FALSE(parser.is_all_found());
}

// ─── Main ─────────────────────────────────────────────────────────────────────

int main() {
    RUN_TEST(test_split_numeric_field);
    RUN_TEST(test_flat_fields_basic);
    RUN_TEST(test_mix_flat_nested_fields);
    RUN_TEST(test_overflow);
    RUN_TEST(test_invalid_json_fields);
    RUN_TEST(test_multi_nested_obj_arrays);
    RUN_TEST(test_short_circuit);
    RUN_TEST(test_object_with_empty_key_path);
    RUN_TEST(test_detect_json_end_no_eos);

    std::cout << "\n" << g_tests_run << " tests run, "
              << g_tests_failed << " failed.\n";
    return g_tests_failed ? 1 : 0;
}
