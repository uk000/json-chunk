package parser

import (
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"sync"
)

var (
	successorsMu sync.Mutex
	successors   = make(map[string]string)
)

// -- helper functions

func ptrString(s string) *string {
	return &s
}

func buildExpectedKV(k string, v interface{}, expected map[string]interface{}) {
	b, _ := json.Marshal(v)
	expected[k] = bytesToValue(b)
}

func feedChunksToParser(cp *ChunkParser, chunks [][]byte) *int {
	total := len(chunks)
	matched := 0
	overflown := 0
	var result *int

	for i, chunk := range chunks {
		fmt.Printf("Processing chunk %d/%d bytes %d\n", i+1, len(chunks), len(chunk))
		cp.ProcessChunk(chunk, i == total-1)
		if len(cp.DoneFields) > matched {
			fmt.Printf("Matched so far: %v\n", cp.DoneFields)
			matched = len(cp.DoneFields)
		}
		if len(cp.OverflowedFields) > overflown {
			fmt.Printf("Overflow so far: %v\n", cp.OverflowedFields)
			overflown = len(cp.OverflowedFields)
		}
		if cp.isAllFound() {
			idx := i + 1
			result = &idx
			return result
		}
	}
	return nil
}

func buildExpectedWithPos(pathMap map[string][2]interface{}, chunks [][]byte) map[string][2]interface{} {
	expected := make(map[string][2]interface{})
	for jsonPath, output := range pathMap {
		fieldsVec := strings.Split(jsonPath, ".")
		rem := []byte{}
		for i, chunk := range chunks {
			data := append([]byte{}, rem...)
			data = append(data, chunk...)
			jsonValue := extractJSONValue(data, fieldsVec)
			if jsonValue != nil {
				outputKey := output[0].(*string)
				if outputKey == nil {
					outputKey = &jsonPath
				}
				expected[*outputKey] = [2]interface{}{i + 1, jsonValue}
				break
			}
			rem = data
		}
	}
	fmt.Println("expected:", len(expected))
	return expected
}

func buildExpected(pathMap map[string][2]interface{}, allExpected map[string]interface{}) map[string]interface{} {
	expected := make(map[string]interface{})
	for jsonPath, output := range pathMap {
		if jsonValue, ok := allExpected[jsonPath]; ok {
			outputKey := output[0].(*string)
			if outputKey == nil {
				outputKey = &jsonPath
			}
			expected[*outputKey] = jsonValue
		}
	}
	return expected
}

func getExpectedFieldNames(pathMap map[string][2]interface{}) ([]string, map[string]string) {
	var fields [][]string
	var allNames []string
	successorMap := make(map[string]string)

	for k := range pathMap {
		fields = append(fields, strings.Split(k, "."))
	}

	for _, f := range fields {
		for _, name := range f {
			if !slices.Contains(allNames, name) {
				allNames = append(allNames, name)
			}
		}
	}

	successorsMu.Lock()
	for _, name := range allNames {
		if val, ok := successors[name]; ok {
			successorMap[name] = val
		}
	}
	successorsMu.Unlock()

	return allNames, successorMap
}

func v(ch rune, valueLen int) string {
	return fmt.Sprintf(`"%s"`, rep(ch, valueLen))
}

func textValue(ch rune, valueLen int, quote rune) string {
	q := string(quote)
	return fmt.Sprintf("%s%s%s", q, rep(ch, valueLen), q)
}

func buildText(fieldCount, valueLen int, quote rune, kvsep string, sep rune, expected *map[string]interface{}) string {
	var flatFields strings.Builder
	q := string(quote)

	for i := 0; i < fieldCount; i++ {
		ch := rune('a' + (i % 26))
		key := fmt.Sprintf("field_%d", i)
		value := rep(ch, valueLen)
		flatFields.WriteString(fmt.Sprintf(`%s%s%s%s%s%s%s`, q, key, q, kvsep, q, value, q))
		flatFields.WriteRune(sep)
		if expected != nil {
			(*expected)[key] = bytesToValue([]byte(fmt.Sprintf(`"%s"`, value)))
		}
	}
	return flatFields.String()
}

func buildFlatFields(fieldCount, valueLen int, expected map[string]interface{}) string {
	return buildText(fieldCount, valueLen, '"', ": ", ',', &expected)
}

func strArray(ch rune, itemCount, valueLen int) string {
	items := make([]string, itemCount)
	for i := 0; i < itemCount; i++ {
		items[i] = v(ch, valueLen)
	}
	return fmt.Sprintf("[ %s ]", strings.Join(items, ","))
}

func objArray(kch, vch rune, itemCount, valueLen int) string {
	items := make([]string, itemCount)
	for i := 0; i < itemCount; i++ {
		items[i] = fmt.Sprintf(`{"name":%s,"value":%s}`, v(kch, valueLen), v(vch, valueLen))
	}
	return fmt.Sprintf("[ %s ]", strings.Join(items, ","))
}

func setLargerJSONSuccessors() {
	successorsMu.Lock()
	defer successorsMu.Unlock()

	m := map[string]string{
		"field_0": "field_1", "field_1": "field_2", "field_2": "field_3",
		"field_3": "field_4", "field_4": "field_5", "field_5": "field_6",
		"field_6": "field_7", "field_7": "field_8", "field_8": "field_9",
		"field_9": "field_10", "field_10": "field_11", "field_11": "field_12",
		"field_12": "field_13", "field_13": "field_14", "field_14": "field_15",
		"field_15": "field_16", "field_16": "field_17", "field_17": "field_18",
		"field_18": "field_19", "field_19": "field_20", "field_20": "field_21",
		"field_21": "field_22", "field_22": "field_23", "field_23": "field_24",
		"field_24": "field_25", "field_25": "field_26", "field_26": "field_27",
		"field_27": "field_28", "field_28": "field_29", "field_29": "field_30",
		"field_30": "field_31", "metadata": "author", "author": "version",
		"version": "stats", "stats": "views", "views": "details",
		"details": "regions", "regions": "locale", "locale": "name",
		"name": "tags", "tags": "items", "items": "config", "config": "key",
		"key": "values", "values": "signature", "signature": "timestamp",
	}
	successors = m
}

func buildLargeJSON(fieldCount, valueLen int, verbose bool, allExpected map[string]interface{}) []byte {
	expected := make(map[string]interface{})
	flatFields := buildFlatFields(fieldCount, valueLen, expected)
	setLargerJSONSuccessors()

	smallJSON := buildSmallJSON(fieldCount, valueLen, allExpected)

	// Remove trailing '}' and leading '{' from smallJSON to embed it
	smallJSONStr := string(smallJSON)
	if len(smallJSONStr) >= 2 {
		smallJSONStr = smallJSONStr[1 : len(smallJSONStr)-1]
	}

	json := fmt.Sprintf(
		"{%s%s}",
		flatFields,
		smallJSONStr,
	)

	jsonBytes := []byte(json)

	// Merge flat fields into allExpected
	for k, v := range expected {
		allExpected[k] = v
	}

	if verbose {
		fmt.Println("=== JSON structure ===")
		printJSONStructure(jsonBytes)
		fmt.Println("=== end structure ===")
	}
	return jsonBytes
}

func buildSmallJSON(fieldCount, valueLen int, allExpected map[string]interface{}) []byte {
	successorsMu.Lock()
	successors = map[string]string{
		"metadata": "author", "author": "version", "version": "stats",
		"stats": "views", "views": "details", "details": "regions",
		"regions": "locale", "locale": "name", "name": "tags",
		"tags": "items", "items": "config", "config": "key",
		"key": "values", "values": "signature", "signature": "timestamp",
	}
	successorsMu.Unlock()

	author := textValue('A', valueLen, ' ')
	version := textValue('B', valueLen, ' ')
	views := objArray('C', 'D', fieldCount, valueLen)
	regions := strArray('E', fieldCount, valueLen)
	locale := textValue('F', valueLen, ' ')
	name := textValue('G', valueLen, ' ')
	tags := strArray('H', fieldCount, valueLen)
	items := objArray('I', 'J', fieldCount, valueLen)
	key := textValue('K', valueLen, ' ')
	value1 := textValue('L', valueLen, ' ')
	value2 := textValue('M', valueLen, ' ')
	signature := strArray('N', fieldCount, valueLen)
	timestamp := 123456789

	// toStr marshals a Go string to a JSON string then unmarshals back, matching
	// Rust's to_json_value(&string) which serializes the String type.
	toStr := func(s string) interface{} {
		b, _ := json.Marshal(s)
		return bytesToValue(b)
	}

	// Format string matches Rust's build_small_json exactly:
	//   - "name" is inside "metadata" (after "stats" closes)
	//   - "", "items", "config", "signature", "timestamp" are all at root level
	jsonStr := fmt.Sprintf(
		`{`+
			` "metadata" :  {`+
			`   ""  :  "%s"  ,`+
			`   "author"  :  "%s"  ,`+
			`"version":"%s",`+
			`"stats":{`+
			`"views":%s,`+
			` "details" :   {  `+
			`  "regions"   :   %s,`+
			`  "locale"   :  "%s"  `+
			`}}`+  // close details, close stats
			`,`+   // comma after stats
			`"name": "%s"`+ // name INSIDE metadata (after stats)
			`},`+  // close metadata, comma
			`"":%s,`+
			`"items":%s,`+
			`"config":{`+
			`"key": "%s",`+
			`"values":{`+
			`"value1": "%s",`+
			`"value2": "%s"`+
			`}}`+ // close values, close config
			`,`+  // comma
			`"signature":%s,`+
			`"timestamp":%d`+
			`}`, // close root
		author, author, version, views, regions, locale, name,
		tags, items, key, value1, value2, signature, timestamp,
	)

	jsonBytes := []byte(jsonStr)

	allExpected["metadata."] = toStr(author)
	allExpected["metadata.author"] = toStr(author)
	allExpected["metadata.version"] = toStr(version)
	allExpected["metadata.stats.views"] = bytesToValue([]byte(views))
	allExpected["metadata.stats.details.regions"] = bytesToValue([]byte(regions))
	allExpected["metadata.stats.details.locale"] = toStr(locale)
	allExpected["metadata.name"] = toStr(name)
	allExpected[""] = bytesToValue([]byte(tags))
	allExpected["items"] = bytesToValue([]byte(items))
	allExpected["config.key"] = toStr(key)
	allExpected["config.values.value1"] = toStr(value1)
	allExpected["config.values.value2"] = toStr(value2)
	allExpected["signature"] = bytesToValue([]byte(signature))
	allExpected["timestamp"] = bytesToValue([]byte(fmt.Sprintf("%d", timestamp)))

	fmt.Println("=== JSON structure ===")
	printJSONStructure(jsonBytes)
	fmt.Println("=== end structure ===")
	return jsonBytes
}

func buildInvalidJSON(fieldCount, valueLen int, allExpected map[string]interface{}) []byte {
	json := buildSmallJSON(fieldCount, valueLen, allExpected)
	json = append(json, []byte(`-------`)...)
	json = append(json, []byte(`+++++++`)...)
	return json
}

func buildTextInput(fieldCount, valueLen int, expected *map[string]interface{}) []byte {
	text := []byte(buildText(fieldCount/3, valueLen, ' ', " ", ' ', expected))
	text = append(text, []byte(`-------`)...)
	text = append(text, []byte(buildText(fieldCount/3, valueLen, ' ', " ", ' ', expected))...)
	text = append(text, []byte(`+++++++`)...)
	text = append(text, []byte(buildText(fieldCount/3, valueLen, ' ', " ", ' ', expected))...)
	return text
}

func randomChunks(bytes []byte, minSize, maxSize int, seed uint64, splitRandomKeys bool, keys []string) [][]byte {
	var chunks [][]byte
	pos := 0
	rng := seed
	lastSplit := false

	for pos < len(bytes) {
		r := maxSize - minSize
		size := minSize + int(lcgNext(&rng)%uint64(maxInt(r, 1)))
		end := minInt(pos+size, len(bytes))
		chunk := bytes[pos:end]

		if splitRandomKeys {
			didSplit := false
			if !lastSplit {
				lastSplit = true
				content := string(chunk)
				for _, key := range keys {
					if idx := strings.Index(content, key); idx >= 0 {
						mid := idx + len(key)/2 + 1
						if mid < len(chunk) {
							chunks = append(chunks, chunk[:mid])
							chunks = append(chunks, chunk[mid:])
							didSplit = true
							break
						}
					}
				}
			} else {
				lastSplit = false
			}
			if !didSplit {
				chunks = append(chunks, chunk)
			}
		} else {
			chunks = append(chunks, chunk)
		}
		pos = end
	}
	return chunks
}

func bytesToValue(b []byte) interface{} {
	var v interface{}
	json.Unmarshal(b, &v)
	return v
}

func extractJSONValue(bytes []byte, path []string) interface{} {
	parser := NewJSONEventGenerator()
	if len(path) == 0 {
		return nil
	}

	cursor := 0
	var pendingKey *string
	matchedDepth := 0
	skippedDepth := 0
	collectingDepth := 0
	collectStart := 0

	for cursor < len(bytes) {
		sliceToParse := bytes[cursor:]
		w := parser.NextEvent(sliceToParse, true)
		cursor += w.ConsumedBytes

		if w.Event == nil {
			if w.ConsumedBytes == 0 {
				break
			}
			continue
		}

		if w.Err != nil {
			break
		}

		ev := w.Event
		if ev.Kind == EventEof {
			break
		}

		if ev.Kind == EventObjectKey {
			if skippedDepth == 0 && collectingDepth == 0 {
				pendingKey = &ev.String
			}
		}

		if ev.Kind == EventStartObject {
			if collectingDepth > 0 {
				collectingDepth++
			} else {
				key := ""
				if pendingKey != nil {
					key = *pendingKey
					pendingKey = nil
				}

				if skippedDepth > 0 {
					skippedDepth++
				} else if key == "" {
					// Root — transparent
				} else if matchedDepth < len(path) && key == path[matchedDepth] {
					if matchedDepth == len(path)-1 {
						collectingDepth = 1
						collectStart = cursor - 1
					} else {
						matchedDepth++
					}
				} else {
					skippedDepth++
				}
			}
		}

		if ev.Kind == EventEndObject {
			if collectingDepth > 0 {
				collectingDepth--
				if collectingDepth == 0 {
					return bytesToValue(bytes[collectStart:cursor])
				}
			} else {
				if skippedDepth > 0 {
					skippedDepth--
				} else if matchedDepth > 0 {
					matchedDepth--
				}
				pendingKey = nil
			}
		}

		if ev.Kind == EventStartArray {
			if collectingDepth > 0 {
				collectingDepth++
			} else {
				if skippedDepth > 0 {
					skippedDepth++
				} else if matchedDepth < len(path) &&
					pendingKey != nil && *pendingKey == path[matchedDepth] &&
					matchedDepth == len(path)-1 {
					collectingDepth = 1
					collectStart = cursor - 1
				} else {
					skippedDepth++
				}
				pendingKey = nil
			}
		}

		if ev.Kind == EventEndArray {
			if collectingDepth > 0 {
				collectingDepth--
				if collectingDepth == 0 {
					return bytesToValue(bytes[collectStart:cursor])
				}
			} else if skippedDepth > 0 {
				skippedDepth--
			}
		}

		if ev.Kind == EventString {
			if collectingDepth == 0 && skippedDepth == 0 &&
				matchedDepth == len(path)-1 &&
				pendingKey != nil && *pendingKey == path[matchedDepth] {
				b, _ := json.Marshal(ev.String)
				return bytesToValue(b)
			}
			if collectingDepth == 0 {
				pendingKey = nil
			}
		}

		if ev.Kind == EventNumber {
			if collectingDepth == 0 && skippedDepth == 0 &&
				matchedDepth == len(path)-1 &&
				pendingKey != nil && *pendingKey == path[matchedDepth] {
				return bytesToValue([]byte(ev.String))
			}
			if collectingDepth == 0 {
				pendingKey = nil
			}
		}

		if ev.Kind == EventBoolean {
			if collectingDepth == 0 && skippedDepth == 0 &&
				matchedDepth == len(path)-1 &&
				pendingKey != nil && *pendingKey == path[matchedDepth] {
				val := "false"
				if ev.Boolean {
					val = "true"
				}
				return bytesToValue([]byte(val))
			}
			if collectingDepth == 0 {
				pendingKey = nil
			}
		}
	}
	return nil
}

func lcgNext(state *uint64) uint64 {
	*state = *state*6364136223846793005 + 1442695040888963407
	return *state
}

func rep(ch rune, length int) string {
	return strings.Repeat(string(ch), length)
}

func minInt(a, b int) int {
	if a < b {
		return a
	}
	return b
}

func maxInt(a, b int) int {
	if a > b {
		return a
	}
	return b
}
