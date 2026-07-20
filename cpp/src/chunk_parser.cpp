#include "chunk_parser.hpp"
#include <cassert>
#include <stdexcept>

// ─── PathTracker::collect ────────────────────────────────────────────────────

void PathTracker::collect(const uint8_t* b, size_t blen, bool is_new) {
    if (overflow) return;
    bool will = false;
    if (is_new) {
        collecting_depth = 1;
        collect_buffer.clear();
        will = true;
    } else if (collecting_depth > 0 && !done) {
        will = true;
    }
    if (will) {
        collect_buffer.insert(collect_buffer.end(), b, b + blen);
        if (max_value_length > 0 && collect_buffer.size() > max_value_length) {
            overflow = true;
        }
    }
}

void PathTracker::collect_start_marker(const uint8_t* b, size_t blen) {
    if (blen == 0) return;
    // Only take the last byte (the '{' or '[' itself, not preceding ':')
    collect(b + blen - 1, 1, true);
}

bool PathTracker::is_array_of_interest() {
    std::string k = current_key.value_or("");
    current_key.reset();

    if (skipped_depth > 0) {
        ++skipped_depth;
    } else if (matched_depth < path_vector.size() && k == path_vector[matched_depth]) {
        if (matched_depth == path_vector.size() - 1) {
            return true;
        }
        ++matched_depth;
        ++array_nesting;
    } else {
        ++skipped_depth;
    }
    return false;
}

bool PathTracker::is_object_of_interest() {
    if (skipped_depth > 0) {
        ++skipped_depth;
    } else if (array_nesting > 0) {
        ++array_nesting;
    } else {
        std::string k = current_key.value_or("");
        current_key.reset();
        if (match_key(k)) return true;
    }
    return false;
}

bool PathTracker::will_collect() const {
    if (skipped_depth == 0 && !path_vector.empty() &&
        matched_depth == path_vector.size() - 1) {
        if (current_key && *current_key == path_vector[matched_depth]) {
            return true;
        }
    }
    return false;
}

void PathTracker::move_collect_pointers(bool is_start_object, bool is_end_object,
                                         bool is_start_array, bool is_end_array) {
    if (is_start_object || is_start_array) {
        ++collecting_depth;
    } else if (is_end_object || is_end_array) {
        --collecting_depth;
    }
}

void PathTracker::unwind(bool array_only) {
    if (skipped_depth > 0) {
        --skipped_depth;
    } else if (array_nesting > 0) {
        --array_nesting;
        if (array_only && array_nesting == 0 && matched_depth > 0) {
            --matched_depth;
        }
    } else if (!array_only && matched_depth > 0) {
        --matched_depth;
    }
}

bool PathTracker::match_key(const std::string& k) {
    if (k.empty() && matched_depth < path_vector.size() && !path_vector[matched_depth].empty()) {
        return false;
    }
    if (matched_depth < path_vector.size() && k == path_vector[matched_depth]) {
        if (matched_depth == path_vector.size() - 1) {
            collecting_depth = 1;
            collect_buffer.clear();
            return true;
        }
        ++matched_depth;
        return false;
    }
    ++skipped_depth;
    return false;
}

void PathTracker::finish() {
    if (!done && !overflow && has_data()) {
        done = true;
    } else if (overflow) {
        reset(true);
    }
}

void PathTracker::reset(bool overflow_flag) {
    matched_depth    = 0;
    array_nesting    = 0;
    skipped_depth    = 0;
    current_key.reset();
    done             = false;
    collecting_depth = 0;
    collect_buffer.clear();
    overflow         = overflow_flag;
}

std::optional<json> PathTracker::get_value() const {
    if (collect_buffer.empty()) return std::nullopt;
    const uint8_t* buf = collect_buffer.data();
    size_t sz = collect_buffer.size();

    try {
        if (buf[0] == '{' || buf[0] == '[') {
            return json::parse(buf, buf + sz);
        }
        // Try JSON parse first (handles numbers, booleans, null)
        auto v = json::parse(buf, buf + sz, nullptr, false);
        if (!v.is_discarded()) return v;
        // Fall back to raw string
        return json(std::string(reinterpret_cast<const char*>(buf), sz));
    } catch (...) {
        return std::string(reinterpret_cast<const char*>(buf), sz);
    }
}

// ─── ChunkParser ─────────────────────────────────────────────────────────────

ChunkParser ChunkParser::make_empty() {
    return ChunkParser{};
}

ChunkParser ChunkParser::new_json_parser(
    const std::unordered_map<std::string, std::pair<std::optional<std::string>, size_t>>& path_map) {
    ChunkParser p;
    p.stop_at_first_match = true;
    for (auto& [path, val] : path_map) {
        p.add_search_field(path, val.first, val.second);
    }
    return p;
}

void ChunkParser::add_search_field(const std::string& json_path,
                                    std::optional<std::string> output,
                                    size_t max_size) {
    PathTracker tracker;
    tracker.path       = json_path;
    tracker.output_key = std::move(output);
    tracker.max_value_length = max_size;

    // Split path by '.'
    std::string part;
    for (char c : json_path) {
        if (c == '.') {
            tracker.path_vector.push_back(part);
            part.clear();
        } else {
            part += c;
        }
    }
    tracker.path_vector.push_back(part);

    tracked_fields[json_path] = std::move(tracker);
}

void ChunkParser::process_chunks(const std::vector<std::vector<uint8_t>>& chunks) {
    for (size_t i = 0; i < chunks.size(); ++i) {
        process_chunk(chunks[i], i == chunks.size() - 1);
        if (is_all_found()) break;
    }
}

void ChunkParser::process_chunk(const std::vector<uint8_t>& chunk, bool eos) {
    scratch_buffer_.insert(scratch_buffer_.end(), chunk.begin(), chunk.end());

    size_t cursor = 0;

    while (true) {
        const uint8_t* slice     = scratch_buffer_.data() + cursor;
        size_t         slice_len = scratch_buffer_.size() - cursor;

        JSONEventWrapper w = json_parser_.next_event(slice, slice_len, eos);

        size_t event_start = cursor;
        cursor += w.consumed_bytes;
        const uint8_t* b    = scratch_buffer_.data() + event_start;
        size_t         blen = cursor - event_start;

        if (!w.event.has_value()) {
            // No event and no error — need more data
            if (blen > 0) feed_trackers(b, blen);
            break;
        }

        if (std::holds_alternative<JSONSyntaxError>(*w.event)) {
            // Syntax error — stop
            break;
        }

        const JSONEvent& jev = std::get<JSONEvent>(*w.event);

        // Convert JSONEvent → ChunkEvent
        ChunkEvent ev;
        switch (jev.kind) {
        case JSONEventKind::String:      ev = {ChunkEventKind::String,      jev.string_value, false}; break;
        case JSONEventKind::Number:      ev = {ChunkEventKind::Number,      jev.string_value, false}; break;
        case JSONEventKind::Boolean:     ev = {ChunkEventKind::Boolean,     {},                jev.bool_value}; break;
        case JSONEventKind::Null:        ev = {ChunkEventKind::Null,        {},                false}; break;
        case JSONEventKind::StartObject: ev = {ChunkEventKind::StartObject, {},                false}; break;
        case JSONEventKind::EndObject:   ev = {ChunkEventKind::EndObject,   {},                false}; break;
        case JSONEventKind::StartArray:  ev = {ChunkEventKind::StartArray,  {},                false}; break;
        case JSONEventKind::EndArray:    ev = {ChunkEventKind::EndArray,    {},                false}; break;
        case JSONEventKind::ObjectKey:   ev = {ChunkEventKind::ObjectKey,   jev.string_value, false}; break;
        case JSONEventKind::Eof:         ev = {ChunkEventKind::Eof,         {},                false}; break;
        }

        json_started = true;

        bool is_start_object = ev.kind == ChunkEventKind::StartObject;
        bool is_end_object   = ev.kind == ChunkEventKind::EndObject;
        bool is_start_array  = ev.kind == ChunkEventKind::StartArray;
        bool is_end_array    = ev.kind == ChunkEventKind::EndArray;
        bool is_eof          = ev.kind == ChunkEventKind::Eof;

        if (is_eof) break;
        if (is_start_object || is_start_array) ++json_depth;
        else if (is_end_object || is_end_array) --json_depth;

        bool has_obj_key = ev.kind == ChunkEventKind::ObjectKey;
        bool is_leaf     = ev.kind == ChunkEventKind::String ||
                           ev.kind == ChunkEventKind::Number ||
                           ev.kind == ChunkEventKind::Boolean;

        std::string leaf_val;
        if (ev.kind == ChunkEventKind::String || ev.kind == ChunkEventKind::Number) {
            leaf_val = ev.string_value;
        } else if (ev.kind == ChunkEventKind::Boolean) {
            leaf_val = ev.bool_value ? "true" : "false";
        }

        for (auto& [key, tracker] : tracked_fields) {
            if (tracker.done) continue;

            if (tracker.is_collecting()) {
                tracker.collect(b, blen, false);
                if (tracker.overflow) {
                    overflowed_fields.insert(tracker.path);
                    tracker.reset(true);
                }
                tracker.move_collect_pointers(is_start_object, is_end_object,
                                               is_start_array, is_end_array);
                if (!tracker.is_collecting()) {
                    tracker.finish();
                    end_tracker(tracker);
                }
                continue;
            }

            if (has_obj_key) {
                if (!tracker.is_skipping()) {
                    tracker.set_current_key(ev.string_value);
                }
            } else if (is_start_object) {
                if (tracker.is_object_of_interest()) {
                    tracker.collect_start_marker(b, blen);
                }
            } else if (is_end_object) {
                tracker.unwind(false);
            } else if (is_start_array) {
                if (tracker.is_array_of_interest()) {
                    tracker.collect_start_marker(b, blen);
                }
            } else if (is_end_array) {
                tracker.unwind(true);
            } else if (is_leaf) {
                if (tracker.will_collect()) {
                    const uint8_t* lv = reinterpret_cast<const uint8_t*>(leaf_val.data());
                    tracker.collect(lv, leaf_val.size(), true);
                    tracker.finish();
                    end_tracker(tracker);
                }
            }
        }

        if (is_all_done()) {
            short_circuit = true;
            break;
        }
    }

    // Drain consumed bytes from scratch buffer
    scratch_buffer_.erase(scratch_buffer_.begin(), scratch_buffer_.begin() + cursor);

    if (json_started && json_depth == 0) end_of_json = true;
    end_of_stream = eos;

    if (eos || short_circuit || end_of_json) {
        end_tracking();
    }
}

void ChunkParser::end_tracking() {
    for (auto& [key, tracker] : tracked_fields) {
        tracker.finish();
        end_tracker(tracker);
    }
}

void ChunkParser::end_tracker(PathTracker& tracker) {
    if (!tracker.overflow) {
        auto val = tracker.get_value();
        if (val) {
            if (tracker.output_key) {
                matches_found_[*tracker.output_key] = std::move(*val);
                matches_found_.erase(tracker.path);
            } else {
                matches_found_[tracker.path] = std::move(*val);
            }
            done_fields.insert(tracker.path);
        }
    } else {
        overflowed_fields.insert(tracker.path);
    }
}

void ChunkParser::feed_trackers(const uint8_t* b, size_t blen) {
    for (auto& [key, tracker] : tracked_fields) {
        tracker.collect(b, blen, false);
        if (tracker.overflow) {
            overflowed_fields.insert(tracker.path);
            tracker.reset(true);
        }
    }
}

bool ChunkParser::is_all_done() const {
    if (!stop_at_first_match) return false;
    for (auto& [k, t] : tracked_fields) {
        if (!t.done && !t.overflow) return false;
    }
    return true;
}

bool ChunkParser::is_all_found() const {
    return tracked_fields.size() == done_fields.size() + overflowed_fields.size();
}

const PathTracker& ChunkParser::get_field(const std::string& name) const {
    auto it = tracked_fields.find(name);
    if (it == tracked_fields.end()) throw std::runtime_error("field not found: " + name);
    return it->second;
}

json ChunkParser::get_result_json() {
    return json(matches_found_);
}
