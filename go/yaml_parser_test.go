package parser

import (
	"encoding/json"
	"testing"
)

func TestJSONProcessChunkScalar(t *testing.T) {
	pathMap := map[string][2]interface{}{
		"a.b": {(*string)(nil), 256},
	}
	cp := NewJSONChunkParser(pathMap)
	cp.ProcessChunk([]byte(`{"a": {"b": 42}}`), true)

	v, ok := cp.MatchesFound["a.b"]
	if !ok {
		t.Fatal("a.b not found")
	}

	var result float64
	if data, err := json.Marshal(v); err == nil {
		json.Unmarshal(data, &result)
		if result != 42 {
			t.Fatalf("Expected 42, got %v", result)
		}
	}
}

func TestYAMLProcessChunkNestedScalar(t *testing.T) {
	yaml := []byte("a:\n  b: 42\n")
	pathMap := map[string][2]interface{}{
		"a.b": {(*string)(nil), 256},
	}
	cp := NewYAMLChunkParser(pathMap)
	cp.ProcessChunk(yaml, true)

	v, ok := cp.MatchesFound["a.b"]
	if !ok {
		t.Fatal("a.b not found in YAML result")
	}

	var result float64
	if data, err := json.Marshal(v); err == nil {
		json.Unmarshal(data, &result)
		if result != 42 {
			t.Fatalf("Expected 42, got %v", result)
		}
	}
}

func TestYAMLProcessChunkStringValue(t *testing.T) {
	yaml := []byte("name: hello\n")
	pathMap := map[string][2]interface{}{
		"name": {(*string)(nil), 256},
	}
	cp := NewYAMLChunkParser(pathMap)
	cp.ProcessChunk(yaml, true)

	v, ok := cp.MatchesFound["name"]
	if !ok {
		t.Fatal("name not found")
	}

	var result string
	if data, err := json.Marshal(v); err == nil {
		json.Unmarshal(data, &result)
		if result != "hello" {
			t.Fatalf("Expected 'hello', got %v", result)
		}
	}
}

func TestYAMLProcessChunkFlowNested(t *testing.T) {
	yaml := []byte("{a: {b: 3}}\n")
	pathMap := map[string][2]interface{}{
		"a.b": {(*string)(nil), 256},
	}
	cp := NewYAMLChunkParser(pathMap)
	cp.ProcessChunk(yaml, true)

	v, ok := cp.MatchesFound["a.b"]
	if !ok {
		t.Fatal("a.b not found")
	}

	var result float64
	if data, err := json.Marshal(v); err == nil {
		json.Unmarshal(data, &result)
		if result != 3 {
			t.Fatalf("Expected 3, got %v", result)
		}
	}
}

func TestYAMLChunkedInput(t *testing.T) {
	chunk1 := []byte("a:\n  b: ")
	chunk2 := []byte("99\n")
	pathMap := map[string][2]interface{}{
		"a.b": {(*string)(nil), 256},
	}
	cp := NewYAMLChunkParser(pathMap)
	cp.ProcessChunk(chunk1, false)
	cp.ProcessChunk(chunk2, true)

	v, ok := cp.MatchesFound["a.b"]
	if !ok {
		t.Fatal("a.b not found")
	}

	var result float64
	if data, err := json.Marshal(v); err == nil {
		json.Unmarshal(data, &result)
		if result != 99 {
			t.Fatalf("Expected 99, got %v", result)
		}
	}
}

func TestTypedMapping(t *testing.T) {
	input := "n: 42\nb: true\ns: hello\nnil: null\n"
	events := collectYAMLEvents(input)

	hasNumber := false
	hasBoolean := false
	hasString := false
	hasNull := false

	for _, ev := range events {
		if ev.Kind == YAMLEventNumber && ev.String == "42" {
			hasNumber = true
		}
		if ev.Kind == YAMLEventBoolean && ev.Boolean {
			hasBoolean = true
		}
		if ev.Kind == YAMLEventString && ev.String == "hello" {
			hasString = true
		}
		if ev.Kind == YAMLEventNull {
			hasNull = true
		}
	}

	if !hasNumber {
		t.Fatal("Number event not found")
	}
	if !hasBoolean {
		t.Fatal("Boolean event not found")
	}
	if !hasString {
		t.Fatal("String event not found")
	}
	if !hasNull {
		t.Fatal("Null event not found")
	}
}

func TestNestedBlock(t *testing.T) {
	yaml := "outer:\n  inner: value\n"
	events := collectYAMLEvents(yaml)

	hasStartObject := false
	hasOuterKey := false
	hasInnerKey := false
	hasValue := false
	endObjectCount := 0

	for _, ev := range events {
		if ev.Kind == YAMLEventStartObject {
			hasStartObject = true
		}
		if ev.Kind == YAMLEventObjectKey && ev.String == "outer" {
			hasOuterKey = true
		}
		if ev.Kind == YAMLEventObjectKey && ev.String == "inner" {
			hasInnerKey = true
		}
		if ev.Kind == YAMLEventString && ev.String == "value" {
			hasValue = true
		}
		if ev.Kind == YAMLEventEndObject {
			endObjectCount++
		}
	}

	if !hasStartObject {
		t.Fatal("StartObject not found")
	}
	if !hasOuterKey {
		t.Fatal("ObjectKey 'outer' not found")
	}
	if !hasInnerKey {
		t.Fatal("ObjectKey 'inner' not found")
	}
	if !hasValue {
		t.Fatal("String 'value' not found")
	}
	if endObjectCount != 2 {
		t.Fatalf("Expected 2 EndObject events, got %d", endObjectCount)
	}
}

func TestFlowNested(t *testing.T) {
	events := collectYAMLEvents("{a: [1, 2], b: 3}\n")

	hasStartObject := false
	hasStartArray := false
	hasNum1 := false
	hasNum2 := false
	hasEndArray := false
	hasKeyB := false
	hasNum3 := false
	hasEndObject := false

	for _, ev := range events {
		if ev.Kind == YAMLEventStartObject {
			hasStartObject = true
		}
		if ev.Kind == YAMLEventStartArray {
			hasStartArray = true
		}
		if ev.Kind == YAMLEventNumber && ev.String == "1" {
			hasNum1 = true
		}
		if ev.Kind == YAMLEventNumber && ev.String == "2" {
			hasNum2 = true
		}
		if ev.Kind == YAMLEventEndArray {
			hasEndArray = true
		}
		if ev.Kind == YAMLEventObjectKey && ev.String == "b" {
			hasKeyB = true
		}
		if ev.Kind == YAMLEventNumber && ev.String == "3" {
			hasNum3 = true
		}
		if ev.Kind == YAMLEventEndObject {
			hasEndObject = true
		}
	}

	if !hasStartObject || !hasStartArray || !hasNum1 || !hasNum2 || !hasEndArray ||
		!hasKeyB || !hasNum3 || !hasEndObject {
		t.Fatal("Missing expected events in flow nested test")
	}
}

func TestSequence(t *testing.T) {
	events := collectYAMLEvents("- 1\n- two\n- true\n")

	hasStartArray := false
	hasNum := false
	hasString := false
	hasBoolean := false
	hasEndArray := false

	for _, ev := range events {
		if ev.Kind == YAMLEventStartArray {
			hasStartArray = true
		}
		if ev.Kind == YAMLEventNumber && ev.String == "1" {
			hasNum = true
		}
		if ev.Kind == YAMLEventString && ev.String == "two" {
			hasString = true
		}
		if ev.Kind == YAMLEventBoolean && ev.Boolean {
			hasBoolean = true
		}
		if ev.Kind == YAMLEventEndArray {
			hasEndArray = true
		}
	}

	if !hasStartArray || !hasNum || !hasString || !hasBoolean || !hasEndArray {
		t.Fatal("Missing expected events in sequence test")
	}
}

func TestQuotedScalarNotTyped(t *testing.T) {
	events := collectYAMLEvents("key: \"true\"\n")

	hasString := false
	hasBoolean := false

	for _, ev := range events {
		if ev.Kind == YAMLEventString && ev.String == "true" {
			hasString = true
		}
		if ev.Kind == YAMLEventBoolean && ev.Boolean {
			hasBoolean = true
		}
	}

	if !hasString {
		t.Fatal("Quoted string 'true' not found")
	}
	if hasBoolean {
		t.Fatal("Should not parse quoted 'true' as boolean")
	}
}

func TestIsYAMLNumber(t *testing.T) {
	tests := []struct {
		input    string
		expected bool
	}{
		{"42", true},
		{"-3.14", true},
		{"1.5e10", true},
		{"0xFF", true},
		{"0o777", true},
		{".inf", true},
		{".nan", true},
		{"true", false},
		{"hello", false},
		{"", false},
		{"1e", false},
	}

	for _, test := range tests {
		result := isYAMLNumber(test.input)
		if result != test.expected {
			t.Fatalf("isYAMLNumber(%q) = %v, expected %v", test.input, result, test.expected)
		}
	}
}

// Helper functions

func collectYAMLEvents(input string) []YAMLEvent {
	parser := NewYAMLEventGenerator()
	buf := []byte(input)
	var events []YAMLEvent

	for {
		w := parser.NextEvent(buf, true)
		if w.Event == nil {
			break
		}

		event := w.Event
		done := event.Kind == YAMLEventEof

		// Convert borrowed strings to owned (deep copy)
		ownedEvent := YAMLEvent{
			Kind:    event.Kind,
			String:  event.String,
			Boolean: event.Boolean,
		}
		events = append(events, ownedEvent)

		if done {
			break
		}
	}

	return events
}
