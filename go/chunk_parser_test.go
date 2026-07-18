package parser

import (
	"encoding/json"
	"fmt"
	"reflect"
	"testing"
)

const (
	MCPMethodJSONPath = "method"
	MCPToolJSONPath   = "params.name"
)

func TestDetectJSONEndWithNoEndOfStream(t *testing.T) {
	allExpected := make(map[string]interface{})
	jsonBytes := buildInvalidJSON(3, 10, allExpected)

	value := "value"
	pathMap := map[string][2]interface{}{
		"config.key":              {(*string)(nil), 100},
		"config.values.xxx":       {&value, 0},
	}

	expected := buildExpected(pathMap, allExpected)
	allNames, successors := getExpectedFieldNames(pathMap)
	chunks := randomChunks(jsonBytes, 50, 300, 42, true, allNames)
	printRelevantChunks(allNames, successors, chunks, 3)

	total := len(chunks)
	parser := NewJSONChunkParser(pathMap)
	for i, chunk := range chunks {
		t.Logf("Processing chunk %d/%d bytes %d", i+1, total, len(chunk))
		parser.ProcessChunk(chunk, false)
		if parser.isAllFound() {
			break
		}
	}

	t.Logf("json_depth = %d", parser.JSONDepth)
	t.Logf("end_of_json = %v", parser.EndOfJSON)
	t.Logf("short_circuit = %v", parser.ShortCircuit)
	t.Logf("end_of_stream = %v", parser.EndOfStream)

	result, _ := parser.GetResultJSON()
	var resultData map[string]interface{}
	json.Unmarshal(result, &resultData)

	printJSONS(expected, parser.MatchesFound, resultData, false)

	if parser.isAllFound() {
		t.Fatal("Expected isAllFound() == false")
	}
}

func TestShortCircuitEarlyFinish(t *testing.T) {
	allExpected := make(map[string]interface{})
	jsonBytes := buildLargeJSON(10, 10, false, allExpected)

	pathMap := map[string][2]interface{}{
		"field_0": {(*string)(nil), 100},
	}
	testHappyPaths(t, jsonBytes, pathMap)

	pathMap = map[string][2]interface{}{
		"field_0": {(*string)(nil), 100},
		"field_9": {(*string)(nil), 100},
	}
	testHappyPaths(t, jsonBytes, pathMap)

	pathMap = map[string][2]interface{}{
		"field_0":                {(*string)(nil), 100},
		"field_9":                {(*string)(nil), 100},
		"metadata.author":        {(*string)(nil), 100},
	}
	testHappyPaths(t, jsonBytes, pathMap)
}

func TestObjectWithEmptyFields(t *testing.T) {
	allExpected := make(map[string]interface{})
	jsonBytes := buildSmallJSON(3, 10, allExpected)

	// Output keys match the Rust test: Some("metadata"), Some("locale"), etc.
	metadata := "metadata"
	locale := "locale"
	value := "value"
	defaultKey := "default"

	pathMap := map[string][2]interface{}{
		"metadata.":                           {&metadata, 100},
		"metadata.stats.details.locale":       {&locale, 100},
		"config.key":                          {(*string)(nil), 100},
		"config.values.value2":                {&value, 0},
		".":                                   {&defaultKey, 100},
	}

	parser := NewJSONChunkParser(pathMap)

	expected := map[string]interface{}{
		"metadata":   extractJSONValue(jsonBytes, []string{"metadata", ""}),
		"locale":     extractJSONValue(jsonBytes, []string{"metadata", "stats", "details", "locale"}),
		"config.key": extractJSONValue(jsonBytes, []string{"config", "key"}),
		"value":      extractJSONValue(jsonBytes, []string{"config", "values", "value2"}),
		// "." path: extractJSONValue uses transparent-root logic, so path [""] finds
		// the root-level "" key directly (matching Rust's extract_json_value(&[""])).
		"default": extractJSONValue(jsonBytes, []string{""}),
	}

	allNames, successors := getExpectedFieldNames(pathMap)
	chunks := randomChunks(jsonBytes, 10, 50, 42, true, allNames)
	printRelevantChunks(allNames, successors, chunks, 3)

	total := len(chunks)
	for i, chunk := range chunks {
		t.Logf("Processing chunk %d/%d bytes %d", i+1, len(chunks), len(chunk))
		parser.ProcessChunk(chunk, i == total-1)
		if parser.isAllFound() {
			break
		}
	}

	result, _ := parser.GetResultJSON()
	var resultData map[string]interface{}
	json.Unmarshal(result, &resultData)

	printJSONS(expected, parser.MatchesFound, resultData, false)

	if !parser.isAllFound() {
		t.Fatal("Expected isAllFound() == true")
	}
	if parser.GetField("metadata.stats.details.locale").Overflow {
		t.Fatal("Expected overflow == false")
	}
	if parser.GetField("config.key").Overflow {
		t.Fatal("Expected overflow == false")
	}
	if parser.GetField("config.values.value2").Overflow {
		t.Fatal("Expected overflow == false")
	}

	// Verify result JSON matches expected
	if !reflect.DeepEqual(resultData, expected) {
		t.Fatalf("Result JSON mismatch.\nExpected: %v\nGot: %v", expected, resultData)
	}
}

func TestFieldsOverflow(t *testing.T) {
	allExpected := make(map[string]interface{})
	jsonBytes := buildLargeJSON(3, 1000, false, allExpected)

	b := "b"
	locale := "locale"
	value := "value"

	pathMap := map[string][2]interface{}{
		"field_1":                            {&b, 0},
		"metadata.stats.details.locale":      {&locale, 200},
		"config.key":                         {(*string)(nil), 100},
		"config.values.value2":               {&value, 0},
	}

	allNames, successors := getExpectedFieldNames(pathMap)
	chunks := randomChunks(jsonBytes, 50, 300, 42, true, allNames)
	expected := buildExpected(pathMap, allExpected)
	printRelevantChunks(allNames, successors, chunks, 3)

	parser := NewJSONChunkParser(pathMap)
	printRelevantChunks(allNames, successors, chunks, 3)
	feedChunksToParser(parser, chunks)

	result, _ := parser.GetResultJSON()
	var resultData map[string]interface{}
	json.Unmarshal(result, &resultData)

	for k := range parser.OverflowedFields {
		delete(expected, k)
		if tracker := parser.GetField(k); tracker.OutputKey != nil {
			delete(expected, *tracker.OutputKey)
		}
	}

	printJSONS(expected, parser.MatchesFound, resultData, false)

	if !parser.isAllFound() {
		t.Fatal("Expected isAllFound() == true")
	}
	if parser.GetField("field_1").Overflow {
		t.Fatal("Expected field_1 overflow == false")
	}
	if !parser.GetField("metadata.stats.details.locale").Overflow {
		t.Fatal("Expected metadata.stats.details.locale overflow == true")
	}
	if !parser.GetField("config.key").Overflow {
		t.Fatal("Expected config.key overflow == true")
	}
	if parser.GetField("config.values.value2").Overflow {
		t.Fatal("Expected config.values.value2 overflow == false")
	}

	// Verify result JSON matches expected
	if !reflect.DeepEqual(resultData, expected) {
		t.Fatalf("Result JSON mismatch.\nExpected: %v\nGot: %v", expected, resultData)
	}
}

func TestSplitNumericField(t *testing.T) {
	jsonStr := fmt.Sprintf(`{"timestamp":%d}`, 123456789)
	jsonBytes := []byte(jsonStr)
	fmt.Println("=== JSON ===")
	printJSONStructure(jsonBytes)

	pathMap := map[string][2]interface{}{
		"timestamp": {(*string)(nil), 100},
	}
	expected := make(map[string]interface{})
	buildExpectedKV("timestamp", 123456789, expected)
	testExpectedHappyPaths(t, jsonBytes, pathMap, expected)
}

func TestMixFlatNestedFieldsInRandomChunks(t *testing.T) {
	allExpected := make(map[string]interface{})
	jsonBytes := buildLargeJSON(10, 50, false, allExpected)

	b := "b"
	locale := "locale"
	value := "value"

	pathMap := map[string][2]interface{}{
		"field_1":                       {&b, 0},
		"metadata.stats.details.locale": {&locale, 100},
		"config.key":                    {(*string)(nil), 100},
		"config.values.value2":          {&value, 0},
		"timestamp":                     {(*string)(nil), 10},
	}
	expected := buildExpected(pathMap, allExpected)
	testExpectedHappyPaths(t, jsonBytes, pathMap, expected)
}

func TestMultiNestedObjArraysInRandomChunks(t *testing.T) {
	allExpected := make(map[string]interface{})
	jsonBytes := buildLargeJSON(2, 10, false, allExpected)

	regions := "regions"
	values := "values"

	pathMap := map[string][2]interface{}{
		"metadata.stats.details.regions": {&regions, 0},
		"config.values":                  {&values, 0},
		"metadata.name":                  {(*string)(nil), 0},
	}
	testHappyPaths(t, jsonBytes, pathMap)
}

func TestInvalidJSONFields(t *testing.T) {
	allExpected := make(map[string]interface{})
	jsonBytes := buildLargeJSON(20, 2000, false, allExpected)

	region := "region"
	pathMap := map[string][2]interface{}{
		"field_x":                           {(*string)(nil), 0},
		"metadata.foo.details.region":       {&region, 512},
		"foo.name":                          {(*string)(nil), 256},
	}

	expected := make(map[string]interface{})
	var allNames []string

	chunks := randomChunks(jsonBytes, 50, 300, 42, false, allNames)
	parser := NewJSONChunkParser(pathMap)
	total := len(chunks)
	for i, chunk := range chunks {
		t.Logf("Processing chunk %d/%d bytes %d", i+1, total, len(chunk))
		parser.ProcessChunk(chunk, i == total-1)
		if parser.isAllFound() {
			break
		}
	}

	result, _ := parser.GetResultJSON()
	var resultData map[string]interface{}
	json.Unmarshal(result, &resultData)

	printJSONS(expected, parser.MatchesFound, resultData, false)

	if parser.isAllFound() {
		t.Fatal("Expected isAllFound() == false")
	}

	// Verify result JSON matches expected
	if !reflect.DeepEqual(resultData, expected) {
		t.Fatalf("Result JSON mismatch.\nExpected: %v\nGot: %v", expected, resultData)
	}
}

func TestMixValidAndInvalidFields(t *testing.T) {
	allExpected := make(map[string]interface{})
	jsonBytes := buildLargeJSON(10, 1000, false, allExpected)

	b := "b"
	locale := "locale"

	pathMap := map[string][2]interface{}{
		"field_x":                       {&b, 1024},
		"metadata.stats.details.locale": {&locale, 100},
		"config.stamp":                  {(*string)(nil), 0},
		"timestamp":                     {(*string)(nil), 0},
	}

	expected := buildExpected(pathMap, allExpected)
	allNames, successors := getExpectedFieldNames(pathMap)
	chunks := randomChunks(jsonBytes, 50, 300, 42, true, allNames)
	printRelevantChunks(allNames, successors, chunks, 3)

	total := len(chunks)
	parser := NewJSONChunkParser(pathMap)
	for i, chunk := range chunks {
		t.Logf("Processing chunk %d/%d bytes %d", i+1, total, len(chunk))
		parser.ProcessChunk(chunk, i == total-1)
		if parser.isAllFound() {
			break
		}
	}

	result, _ := parser.GetResultJSON()
	var resultData map[string]interface{}
	json.Unmarshal(result, &resultData)

	printJSONS(expected, parser.MatchesFound, resultData, false)

	if parser.isAllFound() {
		t.Fatal("Expected isAllFound() == false")
	}
	if !parser.GetField("metadata.stats.details.locale").Overflow {
		t.Fatal("Expected metadata.stats.details.locale overflow == true")
	}
}

func TestFlatTextFields(t *testing.T) {
	allExpected := make(map[string]interface{})
	textBytes := buildTextInput(30, 10, &allExpected)
	pathMap := map[string][2]interface{}{
		"field_0": {(*string)(nil), 0},
		"field_2": {(*string)(nil), 0},
		"field_5": {(*string)(nil), 0},
	}

	allNames, successors := getExpectedFieldNames(pathMap)
	chunks := randomChunks(textBytes, 50, 300, 42, true, allNames)
	printRelevantChunks(allNames, successors, chunks, 3)

	total := len(chunks)
	parser := NewJSONChunkParser(pathMap)
	for i, chunk := range chunks {
		t.Logf("Processing chunk %d/%d bytes %d", i+1, total, len(chunk))
		parser.ProcessChunk(chunk, i == total-1)
		if parser.isAllFound() {
			break
		}
	}

	result, _ := parser.GetResultJSON()
	var resultData map[string]interface{}
	json.Unmarshal(result, &resultData)

	expected := make(map[string]interface{})
	printJSONS(expected, parser.MatchesFound, resultData, false)
}

func testHappyPaths(t *testing.T, jsonBytes []byte, pathMap map[string][2]interface{}) {
	parser := NewJSONChunkParser(pathMap)
	allNames, successors := getExpectedFieldNames(pathMap)
	chunks := randomChunks(jsonBytes, 10, 50, 42, true, allNames)
	expectedWithPos := buildExpectedWithPos(pathMap, chunks)
	printRelevantChunks(allNames, successors, chunks, 3)

	lastChunkO := feedChunksToParser(parser, chunks)
	matches := parser.GetMatches()

	result, _ := parser.GetResultJSON()
	var resultData map[string]interface{}
	json.Unmarshal(result, &resultData)

	printJSONSWithChunkInfo(expectedWithPos, matches, resultData, false)

	if !parser.isAllFound() {
		t.Fatal("Expected isAllFound() == true")
	}
	if lastChunkO == nil {
		t.Fatal("Expected lastChunkO != nil")
	}

	actualLastChunk := *lastChunkO
	t.Logf("Parser exited at chunk %d with %d remaining", actualLastChunk, len(chunks)-actualLastChunk)

	var expectedLastChunk int
	for key, output := range pathMap {
		field := key
		if ptr, ok := output[0].(*string); ok && ptr != nil {
			field = *ptr
		}
		fieldChunk := expectedWithPos[field][0].(int)
		if fieldChunk > actualLastChunk {
			t.Fatalf("Expected field chunk %d <= actual last chunk %d", fieldChunk, actualLastChunk)
		}
		if fieldChunk > expectedLastChunk {
			expectedLastChunk = fieldChunk
		}
	}

	if expectedLastChunk != actualLastChunk {
		t.Fatalf("Expected last chunk %d, got %d", expectedLastChunk, actualLastChunk)
	}
}

func testExpectedHappyPaths(t *testing.T, jsonBytes []byte, pathMap map[string][2]interface{}, expected map[string]interface{}) {
	parser := NewJSONChunkParser(pathMap)
	allNames, successors := getExpectedFieldNames(pathMap)
	chunks := randomChunks(jsonBytes, 10, 50, 42, true, allNames)
	printRelevantChunks(allNames, successors, chunks, 3)

	lastChunkO := feedChunksToParser(parser, chunks)
	matches := parser.GetMatches()

	result, _ := parser.GetResultJSON()
	var resultData map[string]interface{}
	json.Unmarshal(result, &resultData)

	printJSONS(expected, matches, resultData, false)

	if !parser.isAllFound() {
		t.Fatal("Expected isAllFound() == true")
	}
	if lastChunkO == nil {
		t.Fatal("Expected lastChunkO != nil")
	}

	actualLastChunk := *lastChunkO
	t.Logf("Parser exited at chunk %d with %d remaining", actualLastChunk, len(chunks)-actualLastChunk)
}
