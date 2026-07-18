package parser

import (
	"encoding/json"
	"fmt"
	"strings"
)

func printJSONSWithChunkInfo(expected map[string][2]interface{}, matchesFound map[string]interface{}, result interface{}, verbose bool) {
	fmt.Printf("expected (%d):\n", len(expected))
	printJSONChunkInfo(expected, verbose)
	fmt.Printf("parser.matches_found (%d):\n", len(matchesFound))
	printMapJSONSummary(matchesFound, verbose)
	fmt.Println("Result JSON:")
	printJSONSummary(result, verbose)
}

func printJSONS(expected map[string]interface{}, matchesFound map[string]interface{}, result interface{}, verbose bool) {
	fmt.Printf("expected (%d):\n", len(expected))
	printMapJSONSummary(expected, verbose)
	fmt.Printf("parser.matches_found (%d):\n", len(matchesFound))
	printMapJSONSummary(matchesFound, verbose)
	fmt.Println("Result JSON:")
	printJSONSummary(result, verbose)
}

func printJSONStructure(bytes []byte) {
	var stack []bool // true = array, false = object
	var pendingKey *string
	parser := NewJSONEventGenerator()
	cursor := 0

	ind := func(depth int) string {
		return strings.Repeat("  ", depth)
	}

	takeLabel := func() string {
		if pendingKey != nil {
			k := *pendingKey
			pendingKey = nil
			if k == "" {
				k = `""`
			}
			return fmt.Sprintf("%s: ", k)
		}
		if len(stack) > 0 && !stack[len(stack)-1] == false { // is object
			return ""
		}
		if len(stack) > 0 && stack[len(stack)-1] { // is array
			// Just return empty, simplified for brevity
			return ""
		}
		return ""
	}

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
			pendingKey = &ev.String
		}

		if ev.Kind == EventStartObject {
			label := takeLabel()
			fmt.Printf("%s%s{\n", ind(len(stack)), label)
			stack = append(stack, false)
		}

		if ev.Kind == EventEndObject {
			if len(stack) > 0 {
				stack = stack[:len(stack)-1]
			}
			fmt.Printf("%s}\n", ind(len(stack)))
		}

		if ev.Kind == EventStartArray {
			label := takeLabel()
			fmt.Printf("%s%s[\n", ind(len(stack)), label)
			stack = append(stack, true)
		}

		if ev.Kind == EventEndArray {
			if len(stack) > 0 {
				stack = stack[:len(stack)-1]
			}
			fmt.Printf("%s]\n", ind(len(stack)))
		}

		if ev.Kind == EventString {
			label := takeLabel()
			s := ev.String
			length := len(s)
			start := s
			if len(start) > 3 {
				start = start[:3]
			}
			end := s
			if len(end) > 3 {
				end = end[len(end)-3:]
			}
			fmt.Printf("%s%s\"%s...%d...%s\"\n", ind(len(stack)), label, start, length, end)
		}

		if ev.Kind == EventNumber {
			label := takeLabel()
			fmt.Printf("%s%s(number) %s\n", ind(len(stack)), label, ev.String)
		}

		if ev.Kind == EventBoolean {
			label := takeLabel()
			fmt.Printf("%s%s(bool) %v\n", ind(len(stack)), label, ev.Boolean)
		}

		if ev.Kind == EventNull {
			label := takeLabel()
			fmt.Printf("%s%snull\n", ind(len(stack)), label)
		}
	}
}

func findAndPrintKeyInChunk(key string, chunk []byte, nextChunks [][]byte, chunkIdx int, checkOverlap bool, prefix string, searchFrom int) *int {
	if key == "" {
		return nil
	}
	field := "field"
	if prefix != "" {
		field = prefix
	}

	content := string(chunk)
	searchSlice := content
	if searchFrom < len(content) {
		searchSlice = content[searchFrom:]
	} else {
		return nil
	}

	if idx := strings.Index(searchSlice, key); idx >= 0 {
		s := searchFrom + idx
		start := s
		if s >= 10 {
			start = s - 10
		}
		end := s + len(key) + 30
		if end > len(content) {
			end = len(content)
		}
		fmt.Printf("| %-15s | %-20s | %15s | %15s | chunk#%d: ```%s```\n",
			field, key, fmt.Sprintf("%d[%d]", chunkIdx, start), "", chunkIdx, content[start:end])
		result := s + len(key)
		return &result
	}

	if checkOverlap && len(nextChunks) > 0 {
		// Build joined content from current chunk + next chunks
		joined := make([]byte, 0, len(chunk)*2)
		joined = append(joined, chunk...)

		for _, nc := range nextChunks {
			joined = append(joined, nc...)
		}

		if idx := strings.Index(string(joined), key); idx >= 0 {
			s := searchFrom + idx
			start := s
			if s >= 10 {
				start = s - 10
			}
			// Print current chunk
			fmt.Printf("| %-15s | %-20s | %15s | %15s | chunk#%d: ```%s```",
				field, key, fmt.Sprintf("%d[%d]", chunkIdx, start), "", chunkIdx, content)
			// Print overlapping chunks
			for i, nc := range nextChunks {
				fmt.Printf("  >>>>  chunk#%d: ```%s```", chunkIdx+i+1, string(nc))
			}
			fmt.Println()
			result := s + len(key)
			return &result
		}
	}
	return nil
}

func findAndPrintInSingleOrOverlap(key string, chunks [][]byte, chunkIdx, total int, prefix string, depth int) bool {
	chunk := chunks[chunkIdx]
	var nextChunks [][]byte
	for i := 1; i < depth; i++ {
		if chunkIdx+i < total {
			nextChunks = append(nextChunks, chunks[chunkIdx+i])
		}
	}

	searchFrom := 0
	foundAny := false
	for {
		if pos := findAndPrintKeyInChunk(key, chunk, nextChunks, chunkIdx, chunkIdx+1 < total, prefix, searchFrom); pos != nil {
			foundAny = true
			searchFrom = *pos
		} else {
			break
		}
	}
	return foundAny
}

func printRelevantChunks(allNames []string, successorsMap map[string]string, chunks [][]byte, depth int) {
	total := len(chunks)
	for i, c := range chunks {
		fmt.Printf("chunk# %d: ```%s```\n", i, string(c))
	}
	fmt.Printf("\nField distribution across %d chunks for target fields %v:\n", total, allNames)
	var matchedSuccessors []string
	fmt.Printf("| %-15s | %-20s | %15s | %15s | %s\n",
		"kind", "field name", "from chunk", "to chunk", "chunk(s)")
	for i := range chunks {
		for _, name := range allNames {
			if findAndPrintInSingleOrOverlap(name, chunks, i, total, "", depth) {
				if s, ok := successorsMap[name]; ok {
					matchedSuccessors = append(matchedSuccessors, s)
				}
			}
		}
		for j := 0; j < len(matchedSuccessors); j++ {
			if findAndPrintInSingleOrOverlap(matchedSuccessors[j], chunks, i, total, "successor", depth) {
				matchedSuccessors = append(matchedSuccessors[:j], matchedSuccessors[j+1:]...)
				j--
			}
		}
	}
	fmt.Println()
}

func printKV(k string, v interface{}, verbose bool) {
	value, _ := json.Marshal(v)
	valueStr := string(value)
	length := len(valueStr) - 2

	if verbose || length < 10 {
		fmt.Printf("  %s: %s\n", k, valueStr)
		return
	}

	start := valueStr
	if len(start) > 4 {
		start = start[:4]
	}
	end := valueStr
	if len(end) > 4 {
		end = end[len(end)-4:]
	}
	fmt.Printf("  %s: %s...%d...%s\n", k, start, length, end)
}

func printJSONChunkInfo(data map[string][2]interface{}, verbose bool) {
	empty := `""`
	fmt.Println("{")
	for k, v := range data {
		if k == "" {
			k = empty
		}
		chunk := v[0]
		value := v[1]
		fmt.Printf("In chunk# %v\n", chunk)
		printKV(k, value, verbose)
	}
	fmt.Println("}")
}

func printMapJSONSummary(data map[string]interface{}, verbose bool) {
	empty := `""`
	fmt.Println("{")
	for k, v := range data {
		if k == "" {
			k = empty
		}
		printKV(k, v, verbose)
	}
	fmt.Println("}")
}

func printJSONSummary(data interface{}, verbose bool) {
	fmt.Println("{")
	if m, ok := data.(map[string]interface{}); ok {
		for key, value := range m {
			printKV(key, value, verbose)
		}
	} else {
		fmt.Printf("not an object: %v\n", data)
	}
	fmt.Println("}")
}
