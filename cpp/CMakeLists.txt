cmake_minimum_required(VERSION 3.16)
project(json_chunk_cpp CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# ─── nlohmann/json ───────────────────────────────────────────────────────────
# Prefer system/FetchContent, fall back to bundled single-header.
find_package(nlohmann_json QUIET)
if(NOT nlohmann_json_FOUND)
    include(FetchContent)
    FetchContent_Declare(
        nlohmann_json
        GIT_REPOSITORY https://github.com/nlohmann/json.git
        GIT_TAG        v3.11.3
    )
    FetchContent_MakeAvailable(nlohmann_json)
endif()

# ─── Library ─────────────────────────────────────────────────────────────────
add_library(json_chunk_cpp
    src/parser.cpp
    src/chunk_parser.cpp
)
target_include_directories(json_chunk_cpp PUBLIC include)
target_link_libraries(json_chunk_cpp PUBLIC nlohmann_json::nlohmann_json)

# ─── Tests ───────────────────────────────────────────────────────────────────
add_executable(chunk_parser_tests
    tests/chunk_parser_tests.cpp
)
target_include_directories(chunk_parser_tests PRIVATE include tests)
target_link_libraries(chunk_parser_tests PRIVATE json_chunk_cpp)

enable_testing()
add_test(NAME chunk_parser_tests COMMAND chunk_parser_tests)
