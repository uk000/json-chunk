package parser

import (
	"encoding/json"
	"fmt"
	"runtime"
	"strings"
)

// ChunkEventKind represents the kind of normalized chunk event
type ChunkEventKind int

const (
	ChunkEventObjectKey ChunkEventKind = iota
	ChunkEventString
	ChunkEventNumber
	ChunkEventBoolean
	ChunkEventNull
	ChunkEventStartObject
	ChunkEventEndObject
	ChunkEventStartArray
	ChunkEventEndArray
	ChunkEventEof
	ChunkEventIgnored
)

// ChunkEvent is a normalized event from either JSON or YAML parser
type ChunkEvent struct {
	Kind    ChunkEventKind
	String  string
	Boolean bool
}

func fromJSONEvent(ev *JSONEvent) *ChunkEvent {
	switch ev.Kind {
	case EventString:
		return &ChunkEvent{Kind: ChunkEventString, String: ev.String}
	case EventNumber:
		return &ChunkEvent{Kind: ChunkEventNumber, String: ev.String}
	case EventBoolean:
		return &ChunkEvent{Kind: ChunkEventBoolean, Boolean: ev.Boolean}
	case EventNull:
		return &ChunkEvent{Kind: ChunkEventNull}
	case EventStartObject:
		return &ChunkEvent{Kind: ChunkEventStartObject}
	case EventEndObject:
		return &ChunkEvent{Kind: ChunkEventEndObject}
	case EventStartArray:
		return &ChunkEvent{Kind: ChunkEventStartArray}
	case EventEndArray:
		return &ChunkEvent{Kind: ChunkEventEndArray}
	case EventObjectKey:
		return &ChunkEvent{Kind: ChunkEventObjectKey, String: ev.String}
	case EventEof:
		return &ChunkEvent{Kind: ChunkEventEof}
	}
	return nil
}

func fromYAMLEvent(ev *YAMLEvent) *ChunkEvent {
	switch ev.Kind {
	case YAMLEventString:
		return &ChunkEvent{Kind: ChunkEventString, String: ev.String}
	case YAMLEventNumber:
		return &ChunkEvent{Kind: ChunkEventNumber, String: ev.String}
	case YAMLEventBoolean:
		return &ChunkEvent{Kind: ChunkEventBoolean, Boolean: ev.Boolean}
	case YAMLEventNull:
		return &ChunkEvent{Kind: ChunkEventNull}
	case YAMLEventStartObject:
		return &ChunkEvent{Kind: ChunkEventStartObject}
	case YAMLEventEndObject:
		return &ChunkEvent{Kind: ChunkEventEndObject}
	case YAMLEventStartArray:
		return &ChunkEvent{Kind: ChunkEventStartArray}
	case YAMLEventEndArray:
		return &ChunkEvent{Kind: ChunkEventEndArray}
	case YAMLEventObjectKey:
		return &ChunkEvent{Kind: ChunkEventObjectKey, String: ev.String}
	case YAMLEventEof:
		return &ChunkEvent{Kind: ChunkEventEof}
	case YAMLEventStreamStart, YAMLEventStreamEnd, YAMLEventDocumentStart, YAMLEventDocumentEnd:
		return &ChunkEvent{Kind: ChunkEventIgnored}
	}
	return nil
}

func dispatchNextEvent(
	jsonParser *JSONEventGenerator,
	yamlParser *YAMLEventGenerator,
	buf []byte,
	isEnding bool,
) (int, *ChunkEvent, error) {
	if jsonParser != nil {
		w := jsonParser.NextEvent(buf, isEnding)
		if w.Event != nil {
			ev := fromJSONEvent(w.Event)
			return w.ConsumedBytes, ev, w.Err
		}
		return w.ConsumedBytes, nil, w.Err
	}

	if yamlParser != nil {
		w := yamlParser.NextEvent(buf, isEnding)
		if w.Event != nil {
			ev := fromYAMLEvent(w.Event)
			return w.ConsumedBytes, ev, w.Err
		}
		return w.ConsumedBytes, nil, w.Err
	}

	panic("ChunkParser: no parser initialized – call NewJSONChunkParser or NewYAMLChunkParser first")
}

// PathTracker tracks a single search path
type PathTracker struct {
	Path            string
	PathVector      []string
	OutputKey       *string
	MaxValueLength  int
	MatchedDepth    int
	ArrayNesting    int
	SkippedDepth    int
	CurrentKey      *string
	Done            bool
	CollectingDepth int
	CollectBuffer   []byte
	Overflow        bool
}

// ChunkParser is the main orchestrator
type ChunkParser struct {
	ScratchBuffer    []byte
	JSONParser       *JSONEventGenerator
	YAMLParser       *YAMLEventGenerator
	StopAtFirstMatch bool
	TrackedFields    map[string]*PathTracker
	MatchesFound     map[string]interface{}
	DoneFields       map[string]bool
	OverflowedFields map[string]bool
	JSONDepth        int
	JSONStarted      bool
	EndOfJSON        bool
	EndOfStream      bool
	ShortCircuit     bool
}

// NewJSONChunkParser creates a JSON chunk parser
func NewJSONChunkParser(pathMap map[string][2]interface{}) *ChunkParser {
	parser := newChunkParser(pathMap)
	parser.JSONParser = NewJSONEventGenerator()
	return parser
}

// NewYAMLChunkParser creates a YAML chunk parser
func NewYAMLChunkParser(pathMap map[string][2]interface{}) *ChunkParser {
	parser := newChunkParser(pathMap)
	parser.YAMLParser = NewYAMLEventGenerator()
	return parser
}

func newChunkParser(pathMap map[string][2]interface{}) *ChunkParser {
	parser := &ChunkParser{
		ScratchBuffer:    make([]byte, 0),
		StopAtFirstMatch: true,
		TrackedFields:    make(map[string]*PathTracker),
		MatchesFound:     make(map[string]interface{}),
		DoneFields:       make(map[string]bool),
		OverflowedFields: make(map[string]bool),
		JSONDepth:        0,
		JSONStarted:      false,
		EndOfJSON:        false,
		EndOfStream:      false,
		ShortCircuit:     false,
	}

	for path, value := range pathMap {
		outputKey := value[0].(*string)
		maxSize := value[1].(int)
		parser.addSearchField(path, outputKey, maxSize)
	}

	return parser
}

func (cp *ChunkParser) addSearchField(jsonPath string, output *string, maxSize int) {
	parts := strings.Split(jsonPath, ".")
	tracker := &PathTracker{
		Path:           jsonPath,
		PathVector:     parts,
		OutputKey:      output,
		MaxValueLength: maxSize,
		CollectBuffer:  make([]byte, 0),
	}
	cp.TrackedFields[jsonPath] = tracker
}

// ProcessChunks processes multiple chunks
func (cp *ChunkParser) ProcessChunks(chunks [][]byte) {
	for i, chunk := range chunks {
		cp.ProcessChunk(chunk, i == len(chunks)-1)
		if cp.isAllFound() {
			break
		}
	}
}

// ProcessChunk processes a single chunk
func (cp *ChunkParser) ProcessChunk(chunk []byte, endOfStream bool) {
	cp.ScratchBuffer = append(cp.ScratchBuffer, chunk...)

	cursor := 0
	for {
		sliceToParse := cp.ScratchBuffer[cursor:]
		consumedBytes, ev, err := dispatchNextEvent(cp.JSONParser, cp.YAMLParser, sliceToParse, endOfStream)

		eventStart := cursor
		cursor += consumedBytes
		b := cp.ScratchBuffer[eventStart:cursor]

		if ev == nil {
			if err != nil {
				break
			}
			if consumedBytes > 0 {
				cp.feedTrackers(b)
			}
			break
		}
		cp.JSONStarted = true
		if ev.Kind == ChunkEventIgnored {
			if consumedBytes > 0 {
				cp.feedTrackers(b)
			}
			continue
		}

		objKey := (*string)(nil)
		if ev.Kind == ChunkEventObjectKey {
			objKey = &ev.String
		}

		isStartObject := ev.Kind == ChunkEventStartObject
		isEndObject := ev.Kind == ChunkEventEndObject
		isStartArray := ev.Kind == ChunkEventStartArray
		isEndArray := ev.Kind == ChunkEventEndArray
		isEof := ev.Kind == ChunkEventEof

		if isEof {
			break
		} else if isStartObject || isStartArray {
			cp.JSONDepth++
		} else if isEndObject || isEndArray {
			cp.JSONDepth--
			if cp.JSONDepth == 0 {
				dotTracker := cp.TrackedFields["."]
				if dotTracker != nil && !dotTracker.Done && len(dotTracker.CollectBuffer) == 0 {
					fmt.Printf("DEBUG depth0: dot not done yet, buf=empty, b=%q endOfStream=%v\n", string(b), endOfStream)
				}
			}
		}

		for _, tracker := range cp.TrackedFields {
			if tracker.Done {
				continue
			}

			if tracker.isCollecting() {
				tracker.collect(b, false)
				if tracker.Overflow {
					cp.OverflowedFields[tracker.Path] = true
					tracker.reset(true)
				}
				tracker.moveCollectPointers(isStartObject, isEndObject, isStartArray, isEndArray)

				if !tracker.isCollecting() {
					if len(tracker.CollectBuffer) > 0 && tracker.CollectBuffer[0] == '[' {
						fmt.Printf("DEBUG collect-finish: path=%q buf=%q isEndArr=%v isEndObj=%v\n",
							tracker.Path, string(tracker.CollectBuffer), isEndArray, isEndObject)
					}
					tracker.finish()
					cp.endTracker(tracker)
				}
				continue
			}

			if objKey != nil {
				if !tracker.isSkipping() {
					tracker.setCurrentKey(objKey)
				}
			} else if isStartObject {
				if tracker.isObjectOfInterest() {
					tracker.collectStartMarker(b)
				}
			} else if isEndObject {
				tracker.unwind(false)
			} else if isStartArray {
				if tracker.isArrayOfInterest() {
					tracker.collectStartMarker(b)
				}
			} else if isEndArray {
				tracker.unwind(true)
			} else {
				// Handle leaf values (String, Number, Boolean)
				var leafBytes []byte
				switch ev.Kind {
				case ChunkEventString, ChunkEventNumber:
					leafBytes = []byte(ev.String)
				case ChunkEventBoolean:
					leafBytes = []byte(fmt.Sprintf("%v", ev.Boolean))
				default:
					continue
				}

				if tracker.willCollect() {
					tracker.collect(leafBytes, true)
					tracker.finish()
					cp.endTracker(tracker)
				}
			}
		}

		if cp.isAllDone() {
			cp.ShortCircuit = true
			break
		}
	}

	// Drain scratch buffer
	cp.ScratchBuffer = cp.ScratchBuffer[cursor:]
	if cp.JSONStarted && cp.JSONDepth == 0 && !cp.EndOfJSON {
		cp.EndOfJSON = true
		// Check if any tracker is still collecting when EndOfJSON is set
		for _, t := range cp.TrackedFields {
			if !t.Done && len(t.CollectBuffer) > 0 {
				fmt.Printf("DEBUG EndOfJSON set: path=%q buf=%q collectDepth=%d\n", t.Path, string(t.CollectBuffer), t.CollectingDepth)
			}
		}
	}
	cp.EndOfStream = endOfStream
	if endOfStream || cp.ShortCircuit || cp.EndOfJSON {
		// Check if any tracker is mid-collection - for debugging
		for _, t := range cp.TrackedFields {
			if !t.Done && t.CollectingDepth > 0 {
				fmt.Printf("DEBUG pre-endTracking: path=%q depth=%d buf=%q reason: endOfStream=%v shortCircuit=%v endOfJSON=%v jsonDepth=%d\n",
					t.Path, t.CollectingDepth, string(t.CollectBuffer), endOfStream, cp.ShortCircuit, cp.EndOfJSON, cp.JSONDepth)
			}
		}
		cp.endTracking()
	}
}

func (cp *ChunkParser) endTracking() {
	for _, tracker := range cp.TrackedFields {
		if !tracker.Done && len(tracker.CollectBuffer) > 0 {
			fmt.Printf("DEBUG endTracking: path=%q buf=%q depth=%d done=%v\n", tracker.Path, string(tracker.CollectBuffer), tracker.CollectingDepth, tracker.Done)
		}
		tracker.finish()
		cp.endTracker(tracker)
	}
}

func (cp *ChunkParser) endTracker(tracker *PathTracker) {
	if !tracker.Overflow {
		if val := tracker.getValue(); val != nil {
			if tracker.OutputKey != nil {
				cp.MatchesFound[*tracker.OutputKey] = val
				delete(cp.MatchesFound, tracker.Path)
			} else {
				cp.MatchesFound[tracker.Path] = val
			}
			cp.DoneFields[tracker.Path] = true
		}
	} else {
		cp.OverflowedFields[tracker.Path] = true
	}
}

func (cp *ChunkParser) feedTrackers(b []byte) {
	for _, tracker := range cp.TrackedFields {
		tracker.collect(b, false)
		if tracker.Overflow {
			cp.OverflowedFields[tracker.Path] = true
			tracker.reset(true)
		}
	}
}

func (cp *ChunkParser) isAllDone() bool {
	if !cp.StopAtFirstMatch {
		return false
	}
	for _, t := range cp.TrackedFields {
		if !t.Done && !t.Overflow {
			return false
		}
	}
	return true
}

func (cp *ChunkParser) isAllFound() bool {
	return len(cp.TrackedFields) == len(cp.DoneFields)+len(cp.OverflowedFields)
}

func (cp *ChunkParser) GetField(name string) *PathTracker {
	if t, ok := cp.TrackedFields[name]; ok {
		return t
	}
	panic("field not found")
}

func (cp *ChunkParser) GetMatches() map[string]interface{} {
	return cp.MatchesFound
}

func (cp *ChunkParser) GetResultJSON() (json.RawMessage, error) {
	data, err := json.Marshal(cp.MatchesFound)
	if err != nil {
		return nil, err
	}
	return json.RawMessage(data), nil
}

// PathTracker methods

func (pt *PathTracker) isCollecting() bool {
	return pt.CollectingDepth > 0
}

func (pt *PathTracker) isSkipping() bool {
	return pt.SkippedDepth > 0
}

func (pt *PathTracker) setCurrentKey(key *string) {
	pt.CurrentKey = key
}

func (pt *PathTracker) collect(b []byte, isNew bool) {
	if pt.Overflow {
		return
	}

	willCollect := false
	if isNew {
		pt.CollectingDepth = 1
		pt.CollectBuffer = make([]byte, 0)
		willCollect = true
	} else if pt.CollectingDepth > 0 && !pt.Done {
		willCollect = true
	}

	if willCollect {
		pt.CollectBuffer = append(pt.CollectBuffer, b...)
		if pt.MaxValueLength > 0 && len(pt.CollectBuffer) > pt.MaxValueLength {
			pt.Overflow = true
		}
	}
}

func (pt *PathTracker) collectStartMarker(b []byte) {
	if len(b) > 0 {
		pt.collect(b[len(b)-1:len(b)], true)
	}
}

func (pt *PathTracker) isArrayOfInterest() bool {
	k := ""
	if pt.CurrentKey != nil {
		k = *pt.CurrentKey
	}
	pt.CurrentKey = nil

	if pt.SkippedDepth > 0 {
		pt.SkippedDepth++
	} else if pt.MatchedDepth < len(pt.PathVector) && k == pt.PathVector[pt.MatchedDepth] {
		if pt.MatchedDepth == len(pt.PathVector)-1 {
			return true
		}
		pt.MatchedDepth++
		pt.ArrayNesting++
	} else {
		pt.SkippedDepth++
	}
	return false
}

func (pt *PathTracker) isObjectOfInterest() bool {
	if pt.SkippedDepth > 0 {
		pt.SkippedDepth++
	} else if pt.ArrayNesting > 0 {
		pt.ArrayNesting++
	} else {
		k := ""
		if pt.CurrentKey != nil {
			k = *pt.CurrentKey
		}
		pt.CurrentKey = nil
		if pt.matchKey(k) {
			return true
		}
	}
	return false
}

func (pt *PathTracker) willCollect() bool {
	if pt.SkippedDepth == 0 && len(pt.PathVector) > 0 && pt.MatchedDepth == len(pt.PathVector)-1 {
		if pt.CurrentKey != nil && *pt.CurrentKey == pt.PathVector[pt.MatchedDepth] {
			return true
		}
	}
	return false
}

func (pt *PathTracker) moveCollectPointers(isStartObject, isEndObject, isStartArray, isEndArray bool) {
	if isStartObject || isStartArray {
		pt.CollectingDepth++
	} else if isEndObject || isEndArray {
		pt.CollectingDepth--
	}
}

func (pt *PathTracker) unwind(arrayOnly bool) {
	if pt.SkippedDepth > 0 {
		pt.SkippedDepth--
	} else if pt.ArrayNesting > 0 {
		pt.ArrayNesting--
		if arrayOnly && pt.ArrayNesting == 0 && pt.MatchedDepth > 0 {
			pt.MatchedDepth--
		}
	} else if !arrayOnly && pt.MatchedDepth > 0 {
		pt.MatchedDepth--
	}
}

func (pt *PathTracker) matchKey(k string) bool {
	if k == "" && pt.MatchedDepth < len(pt.PathVector) && pt.PathVector[pt.MatchedDepth] != "" {
		return false
	}

	if pt.MatchedDepth < len(pt.PathVector) && k == pt.PathVector[pt.MatchedDepth] {
		if pt.MatchedDepth == len(pt.PathVector)-1 {
			pt.CollectingDepth = 1
			pt.CollectBuffer = make([]byte, 0)
			return true
		}
		pt.MatchedDepth++
		return false
	}

	pt.SkippedDepth++
	return false
}

func (pt *PathTracker) finish() {
	if !pt.Done && !pt.Overflow && pt.hasData() {
		pcs := make([]uintptr, 5)
		n := runtime.Callers(2, pcs)
		frames := runtime.CallersFrames(pcs[:n])
		var callers []string
		for {
			f, more := frames.Next()
			callers = append(callers, fmt.Sprintf("%s:%d", f.Function, f.Line))
			if !more {
				break
			}
		}
		fmt.Printf("DEBUG finish: path=%q buf=%q depth=%d callers=%v\n", pt.Path, string(pt.CollectBuffer), pt.CollectingDepth, callers)
		pt.Done = true
	} else if pt.Overflow {
		pt.reset(true)
	}
}

func (pt *PathTracker) hasData() bool {
	return len(pt.CollectBuffer) > 0
}

func (pt *PathTracker) getValue() interface{} {
	if len(pt.CollectBuffer) == 0 {
		return nil
	}

	// Try to parse as JSON first
	var jsonValue interface{}
	err := json.Unmarshal(pt.CollectBuffer, &jsonValue)
	if err == nil {
		return jsonValue
	}

	// If not JSON, treat as string
	return string(pt.CollectBuffer)
}

func (pt *PathTracker) reset(overflow bool) {
	pt.MatchedDepth = 0
	pt.ArrayNesting = 0
	pt.SkippedDepth = 0
	pt.CurrentKey = nil
	pt.Done = false
	pt.CollectingDepth = 0
	pt.CollectBuffer = make([]byte, 0)
	pt.Overflow = overflow
}
