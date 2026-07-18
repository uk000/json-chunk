package parser

import (
	"fmt"
	"strings"
)

const MaxIndentStackSize = 65_536
const MaxIndentSentinel = 1 << 30

// YAMLScalarStyle represents how a scalar was written in YAML source
type YAMLScalarStyle int

const (
	YAMLScalarPlain YAMLScalarStyle = iota
	YAMLScalarSingleQuoted
	YAMLScalarDoubleQuoted
	YAMLScalarLiteral
	YAMLScalarFolded
)

// YAMLEventKind represents the kind of YAML event
type YAMLEventKind int

const (
	YAMLEventString YAMLEventKind = iota
	YAMLEventNumber
	YAMLEventBoolean
	YAMLEventNull
	YAMLEventStartArray
	YAMLEventEndArray
	YAMLEventStartObject
	YAMLEventEndObject
	YAMLEventObjectKey
	YAMLEventEof
	YAMLEventStreamStart
	YAMLEventStreamEnd
	YAMLEventDocumentStart
	YAMLEventDocumentEnd
)

// YAMLEvent represents a parsed YAML event
type YAMLEvent struct {
	Kind    YAMLEventKind
	String  string
	Boolean bool
}

// YAMLEventWrapper wraps a YAML event with metadata
type YAMLEventWrapper struct {
	ConsumedBytes int
	Event         *YAMLEvent
	Err           error
}

// YAMLSyntaxError represents a YAML syntax error
type YAMLSyntaxError struct {
	Location TextPosition
	Message  string
}

// Error implements the error interface
func (e *YAMLSyntaxError) Error() string {
	return fmt.Sprintf("YAML parse error at line %d column %d: %s",
		e.Location.Line+1, e.Location.Column+1, e.Message)
}

// BlockContext represents the type of block collection
type blockContextKind int

const (
	blockContextMapping blockContextKind = iota
	blockContextSequence
)

// IndentEntry tracks an open block collection
type indentEntry struct {
	indent  int
	context blockContextKind
}

// Chomp mode for block scalars
type chompKind int

const (
	chompStrip chompKind = iota
	chompClip
	chompKeep
)

// ParseStateKind represents YAML parser state
type parseStateKind int

const (
	parseStateStreamNotStarted parseStateKind = iota
	parseStateBeforeDocument
	parseStateBlockNode
	parseStateBlockMappingKey
	parseStateBlockMappingValue
	parseStateBlockSequenceEntry
	parseStateBlockScalarContent
	parseStateFlowMappingKey
	parseStateFlowMappingColon
	parseStateFlowMappingValue
	parseStateFlowMappingCommaOrEnd
	parseStateFlowSequenceEntry
	parseStateFlowSequenceCommaOrEnd
	parseStateDone
)

// ParseState holds the current parser state with all data
type parseState struct {
	kind          parseStateKind
	minIndent     int
	indent        int
	mappingIndent int
	scalarStyle   YAMLScalarStyle
	contentIndent int
	chomp         chompKind
}

// YAMLEventGenerator is the main YAML parser
type YAMLEventGenerator struct {
	fileOffset          int64
	fileLine            int64
	fileStartOfLastLine int64
	state               parseState
	indentStack         []indentEntry
	pendingEvents       []*YAMLEvent
	flowReturnStates    []parseState
	maxIndentStackSize  int
	blockScalarBuf      strings.Builder
}

// NewYAMLEventGenerator creates a new YAML parser
func NewYAMLEventGenerator() *YAMLEventGenerator {
	return &YAMLEventGenerator{
		fileOffset:          0,
		fileLine:            0,
		fileStartOfLastLine: 0,
		state: parseState{
			kind:          parseStateStreamNotStarted,
			minIndent:     0,
			indent:        0,
			mappingIndent: 0,
			contentIndent: 0,
		},
		indentStack:        make([]indentEntry, 0),
		pendingEvents:      make([]*YAMLEvent, 0),
		flowReturnStates:   make([]parseState, 0),
		maxIndentStackSize: MaxIndentStackSize,
	}
}

// WithMaxIndentStackSize sets the maximum nesting depth
func (g *YAMLEventGenerator) WithMaxIndentStackSize(size int) *YAMLEventGenerator {
	g.maxIndentStackSize = size
	return g
}

// NextEvent reads the next YAML event
func (g *YAMLEventGenerator) NextEvent(inputBuffer []byte, isEnding bool) YAMLEventWrapper {
	if len(g.pendingEvents) > 0 {
		event := g.pendingEvents[0]
		g.pendingEvents = g.pendingEvents[1:]
		return YAMLEventWrapper{
			ConsumedBytes: 0,
			Event:         event,
			Err:           nil,
		}
	}

	if g.state.kind == parseStateStreamNotStarted {
		g.state.kind = parseStateBeforeDocument
		return YAMLEventWrapper{
			ConsumedBytes: 0,
			Event:         &YAMLEvent{Kind: YAMLEventStreamStart},
			Err:           nil,
		}
	}

	if g.state.kind == parseStateDone {
		return YAMLEventWrapper{
			ConsumedBytes: 0,
			Event:         &YAMLEvent{Kind: YAMLEventEof},
			Err:           nil,
		}
	}

	base := g.fileOffset
	event, err, ok := g.scan(inputBuffer, base, isEnding)

	consumed := int(g.fileOffset - base)
	if consumed < 0 {
		consumed = 0
	}

	if !ok && err == nil {
		return YAMLEventWrapper{
			ConsumedBytes: consumed,
			Event:         nil,
			Err:           nil,
		}
	}

	return YAMLEventWrapper{
		ConsumedBytes: consumed,
		Event:         event,
		Err:           err,
	}
}

func (g *YAMLEventGenerator) scan(buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	switch g.state.kind {
	case parseStateStreamNotStarted, parseStateDone:
		return nil, nil, false
	case parseStateBeforeDocument:
		return g.scanBeforeDocument(buf, base, isEnding)
	case parseStateBlockNode:
		return g.scanBlockNode(buf, base, isEnding, g.state.minIndent)
	case parseStateBlockMappingKey:
		return g.scanBlockMappingKey(buf, base, isEnding, g.state.indent)
	case parseStateBlockMappingValue:
		return g.scanBlockMappingValue(buf, base, isEnding, g.state.mappingIndent)
	case parseStateBlockSequenceEntry:
		return g.scanBlockSequenceEntry(buf, base, isEnding, g.state.indent)
	case parseStateBlockScalarContent:
		return g.scanBlockScalarContent(buf, base, isEnding, g.state.scalarStyle, g.state.contentIndent, g.state.chomp)
	case parseStateFlowMappingKey, parseStateFlowMappingColon, parseStateFlowMappingValue, parseStateFlowMappingCommaOrEnd,
		parseStateFlowSequenceEntry, parseStateFlowSequenceCommaOrEnd:
		return g.scanFlow(buf, base, isEnding)
	}
	return nil, nil, false
}

func (g *YAMLEventGenerator) scanBeforeDocument(buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if !g.skipBlockWhitespace(buf, base, isEnding) {
		return nil, nil, false
	}

	slice := g.cur(buf, base)
	if len(slice) == 0 {
		g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventStreamEnd})
		g.state.kind = parseStateDone
		return &YAMLEvent{Kind: YAMLEventEof}, nil, true
	}

	if g.peekDocMarker(slice, "---", isEnding) {
		g.consumeLine(buf, base, isEnding)
		g.state.kind = parseStateBlockNode
		g.state.minIndent = 0
		return &YAMLEvent{Kind: YAMLEventDocumentStart}, nil, true
	}

	g.state.kind = parseStateBlockNode
	g.state.minIndent = 0
	return &YAMLEvent{Kind: YAMLEventDocumentStart}, nil, true
}

func (g *YAMLEventGenerator) scanBlockNode(buf []byte, base int64, isEnding bool, minIndent int) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if !g.skipBlockWhitespace(buf, base, isEnding) {
		return nil, nil, false
	}

	slice := g.cur(buf, base)
	if len(slice) == 0 {
		return g.closeAllThenEof(isEnding)
	}

	if g.peekDocMarker(slice, "...", isEnding) || g.peekDocMarker(slice, "---", isEnding) {
		g.state = g.blockReturnStateAfterValue()
		return &YAMLEvent{Kind: YAMLEventNull}, nil, true
	}

	indent := measureIndent(slice)
	if indent < minIndent {
		g.restoreBlockStateAfterValue()
		return &YAMLEvent{Kind: YAMLEventNull}, nil, true
	}

	content := slice[indent:]

	if len(content) > 0 && content[0] == '{' {
		g.advance(indent + 1)
		g.pushFlowReturn(g.blockReturnStateAfterValue())
		g.state.kind = parseStateFlowMappingKey
		return &YAMLEvent{Kind: YAMLEventStartObject}, nil, true
	}

	if len(content) > 0 && content[0] == '[' {
		g.advance(indent + 1)
		g.pushFlowReturn(g.blockReturnStateAfterValue())
		g.state.kind = parseStateFlowSequenceEntry
		return &YAMLEvent{Kind: YAMLEventStartArray}, nil, true
	}

	if isSeqEntry(content) {
		if len(g.indentStack) >= g.maxIndentStackSize {
			return nil, g.makeError("Max nesting depth reached"), true
		}
		g.indentStack = append(g.indentStack, indentEntry{indent: indent, context: blockContextSequence})
		g.state.kind = parseStateBlockSequenceEntry
		g.state.indent = indent
		return &YAMLEvent{Kind: YAMLEventStartArray}, nil, true
	}

	if findBlockMappingColon(content) >= 0 {
		if len(g.indentStack) >= g.maxIndentStackSize {
			return nil, g.makeError("Max nesting depth reached"), true
		}
		g.indentStack = append(g.indentStack, indentEntry{indent: indent, context: blockContextMapping})
		g.state.kind = parseStateBlockMappingKey
		g.state.indent = indent
		return &YAMLEvent{Kind: YAMLEventStartObject}, nil, true
	}

	g.advance(indent)
	return g.scanScalarAsValue(buf, base, isEnding, false)
}

func (g *YAMLEventGenerator) scanBlockMappingKey(buf []byte, base int64, isEnding bool, indent int) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if !g.skipBlockWhitespace(buf, base, isEnding) {
		return nil, nil, false
	}

	slice := g.cur(buf, base)
	if len(slice) == 0 {
		return g.closeAllThenEof(isEnding)
	}

	if g.peekDocMarker(slice, "...", isEnding) || g.peekDocMarker(slice, "---", isEnding) {
		return g.closeBlocksForDocMarker(buf, base, isEnding)
	}

	lineIndent := measureIndent(slice)
	if lineIndent < indent {
		return g.closeBlocksToIndent(lineIndent, buf, base, isEnding)
	}

	content := slice[lineIndent:]
	colonPos := findBlockMappingColon(content)
	if colonPos < 0 {
		err := g.makeError("Expected a mapping key (key:) at this indent")
		g.advanceToNextLine(buf, base, isEnding)
		return nil, err, true
	}

	g.advance(lineIndent + colonPos)
	keyStart := int(g.fileOffset-base) - colonPos
	keyEnd := int(g.fileOffset - base)
	g.advance(1)

	rawKey := strings.TrimRight(string(buf[keyStart:keyEnd]), " \t")
	key := rawKey

	after := g.cur(buf, base)
	hasInline := len(after) > 0 && after[0] != '\n' && after[0] != '\r' && after[0] != '#'

	if hasInline {
		if len(after) > 0 && (after[0] == ' ' || after[0] == '\t') {
			g.advance(1)
		}
	} else {
		g.advanceToNextLine(buf, base, isEnding)
	}

	g.state.kind = parseStateBlockMappingValue
	g.state.mappingIndent = indent
	return &YAMLEvent{Kind: YAMLEventObjectKey, String: key}, nil, true
}

func (g *YAMLEventGenerator) scanBlockMappingValue(buf []byte, base int64, isEnding bool, mappingIndent int) (*YAMLEvent, *YAMLSyntaxError, bool) {
	inline := g.cur(buf, base)
	hasInline := len(inline) > 0 && inline[0] != '\n' && inline[0] != '\r' && inline[0] != '#'

	if hasInline {
		return g.scanValueOrNested(buf, base, isEnding, mappingIndent)
	}

	if !g.skipBlockWhitespace(buf, base, isEnding) {
		return nil, nil, false
	}

	slice := g.cur(buf, base)
	if len(slice) == 0 {
		if !isEnding {
			return nil, nil, false
		}
		g.state.kind = parseStateBlockMappingKey
		g.state.indent = mappingIndent
		g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventEndObject})
		return &YAMLEvent{Kind: YAMLEventNull}, nil, true
	}

	if g.peekDocMarker(slice, "...", isEnding) || g.peekDocMarker(slice, "---", isEnding) {
		g.state.kind = parseStateBlockMappingKey
		g.state.indent = mappingIndent
		return &YAMLEvent{Kind: YAMLEventNull}, nil, true
	}

	lineIndent := measureIndent(slice)
	if lineIndent <= mappingIndent {
		g.state.kind = parseStateBlockMappingKey
		g.state.indent = mappingIndent
		return &YAMLEvent{Kind: YAMLEventNull}, nil, true
	}

	return g.scanValueOrNested(buf, base, isEnding, mappingIndent)
}

func (g *YAMLEventGenerator) scanValueOrNested(buf []byte, base int64, isEnding bool, parentIndent int) (*YAMLEvent, *YAMLSyntaxError, bool) {
	slice := g.cur(buf, base)

	if len(slice) == 0 {
		if !isEnding {
			return nil, nil, false
		}
		g.restoreBlockStateAfterValue()
		return &YAMLEvent{Kind: YAMLEventNull}, nil, true
	}

	indent := measureIndent(slice)
	content := slice[indent:]

	if style, chomp := isBlockScalarIndicator(content); style != -1 {
		g.advance(indent + 1)
		g.consumeBlockScalarHeader(buf, base, isEnding)
		g.blockScalarBuf.Reset()
		g.state.kind = parseStateBlockScalarContent
		g.state.scalarStyle = style
		g.state.contentIndent = MaxIndentSentinel
		g.state.chomp = chomp
		return g.scanBlockScalarContent(buf, base, isEnding, style, MaxIndentSentinel, chomp)
	}

	if len(content) > 0 && content[0] == '{' {
		g.advance(indent + 1)
		ret := g.blockReturnStateAfterValue()
		g.pushFlowReturn(ret)
		g.state.kind = parseStateFlowMappingKey
		return &YAMLEvent{Kind: YAMLEventStartObject}, nil, true
	}

	if len(content) > 0 && content[0] == '[' {
		g.advance(indent + 1)
		ret := g.blockReturnStateAfterValue()
		g.pushFlowReturn(ret)
		g.state.kind = parseStateFlowSequenceEntry
		return &YAMLEvent{Kind: YAMLEventStartArray}, nil, true
	}

	if isSeqEntry(content) {
		if len(g.indentStack) >= g.maxIndentStackSize {
			return nil, g.makeError("Max nesting depth reached"), true
		}
		g.indentStack = append(g.indentStack, indentEntry{indent: indent, context: blockContextSequence})
		g.state.kind = parseStateBlockSequenceEntry
		g.state.indent = indent
		return &YAMLEvent{Kind: YAMLEventStartArray}, nil, true
	}

	if findBlockMappingColon(content) >= 0 {
		if len(g.indentStack) >= g.maxIndentStackSize {
			return nil, g.makeError("Max nesting depth reached"), true
		}
		g.indentStack = append(g.indentStack, indentEntry{indent: indent, context: blockContextMapping})
		g.state.kind = parseStateBlockMappingKey
		g.state.indent = indent
		return &YAMLEvent{Kind: YAMLEventStartObject}, nil, true
	}

	g.advance(indent)
	return g.scanScalarAsValue(buf, base, isEnding, false)
}

func (g *YAMLEventGenerator) scanBlockSequenceEntry(buf []byte, base int64, isEnding bool, indent int) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if !g.skipBlockWhitespace(buf, base, isEnding) {
		return nil, nil, false
	}

	slice := g.cur(buf, base)
	if len(slice) == 0 {
		return g.closeAllThenEof(isEnding)
	}

	if g.peekDocMarker(slice, "...", isEnding) || g.peekDocMarker(slice, "---", isEnding) {
		return g.closeBlocksForDocMarker(buf, base, isEnding)
	}

	lineIndent := measureIndent(slice)
	if lineIndent < indent {
		return g.closeBlocksToIndent(lineIndent, buf, base, isEnding)
	}

	content := slice[lineIndent:]
	if !isSeqEntry(content) {
		err := g.makeError("Expected a sequence entry `- ` at this indent")
		g.advanceToNextLine(buf, base, isEnding)
		return nil, err, true
	}

	g.advance(lineIndent + 1)
	if len(g.cur(buf, base)) > 0 && (g.cur(buf, base)[0] == ' ' || g.cur(buf, base)[0] == '\t') {
		g.advance(1)
	}

	valSlice := g.cur(buf, base)
	noInline := len(valSlice) == 0 || (len(valSlice) > 0 && (valSlice[0] == '\n' || valSlice[0] == '\r' || valSlice[0] == '#'))

	if noInline {
		g.advanceToNextLine(buf, base, isEnding)
		return g.scan(buf, base, isEnding)
	}

	return g.scanValueOrNested(buf, base, isEnding, indent)
}

func (g *YAMLEventGenerator) scanBlockScalarContent(buf []byte, base int64, isEnding bool, style YAMLScalarStyle, contentIndent int, chomp chompKind) (*YAMLEvent, *YAMLSyntaxError, bool) {
	for {
		slice := g.cur(buf, base)
		lineEnd := findNewline(slice)
		if lineEnd < 0 {
			if isEnding {
				lineEnd = len(slice)
			} else {
				return nil, nil, false
			}
		}

		line := slice[:lineEnd]
		isBlank := true
		for _, b := range line {
			if b != ' ' && b != '\t' {
				isBlank = false
				break
			}
		}

		if isBlank {
			g.blockScalarBuf.WriteRune('\n')
			g.consumeBytesAndNewline(buf, base, lineEnd)
			continue
		}

		lineIndentVal := measureIndent(line)
		if contentIndent == MaxIndentSentinel {
			contentIndent = lineIndentVal
			g.state.contentIndent = lineIndentVal
		}

		if lineIndentVal < contentIndent {
			break
		}

		text := string(line[contentIndent:])
		switch style {
		case YAMLScalarLiteral:
			g.blockScalarBuf.WriteString(text)
			g.blockScalarBuf.WriteRune('\n')
		case YAMLScalarFolded:
			if g.blockScalarBuf.Len() > 0 && !strings.HasSuffix(g.blockScalarBuf.String(), "\n") {
				g.blockScalarBuf.WriteRune(' ')
			}
			g.blockScalarBuf.WriteString(text)
		}
		g.consumeBytesAndNewline(buf, base, lineEnd)
	}

	result := applyChomp(g.blockScalarBuf.String(), chomp)
	g.blockScalarBuf.Reset()
	g.restoreBlockStateAfterValue()

	return &YAMLEvent{Kind: YAMLEventString, String: result}, nil, true
}

func (g *YAMLEventGenerator) scanFlow(buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	g.skipFlowWhitespace(buf, base)

	slice := g.cur(buf, base)
	if len(slice) == 0 {
		if isEnding {
			return nil, g.makeError("Unexpected end of input inside flow collection"), true
		}
		return nil, nil, false
	}

	switch g.state.kind {
	case parseStateFlowMappingKey:
		return g.scanFlowMappingKey(buf, base, isEnding)
	case parseStateFlowMappingColon:
		return g.scanFlowMappingColon(buf, base, isEnding)
	case parseStateFlowMappingValue:
		return g.scanFlowMappingValue(buf, base, isEnding)
	case parseStateFlowMappingCommaOrEnd:
		return g.scanFlowMappingCommaOrEnd(buf, base, isEnding)
	case parseStateFlowSequenceEntry:
		return g.scanFlowSequenceEntry(buf, base, isEnding)
	case parseStateFlowSequenceCommaOrEnd:
		return g.scanFlowSequenceCommaOrEnd(buf, base, isEnding)
	}
	return nil, nil, false
}

func (g *YAMLEventGenerator) scanFlowMappingKey(buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if len(g.cur(buf, base)) > 0 && g.cur(buf, base)[0] == '}' {
		g.advance(1)
		ret := g.popFlowReturn()
		g.state = ret
		return &YAMLEvent{Kind: YAMLEventEndObject}, nil, true
	}

	val, _, ok := g.readFlowScalar(buf, base, isEnding)
	if !ok {
		return nil, nil, false
	}

	g.state.kind = parseStateFlowMappingColon
	return &YAMLEvent{Kind: YAMLEventObjectKey, String: val}, nil, true
}

func (g *YAMLEventGenerator) scanFlowMappingColon(buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if len(g.cur(buf, base)) == 0 || g.cur(buf, base)[0] != ':' {
		return nil, g.makeError("Expected ':' after flow mapping key"), true
	}
	g.advance(1)
	g.skipFlowWhitespace(buf, base)
	g.state.kind = parseStateFlowMappingValue
	return g.scanFlowMappingValue(buf, base, isEnding)
}

func (g *YAMLEventGenerator) scanFlowMappingValue(buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	slice := g.cur(buf, base)
	if len(slice) > 0 && slice[0] == '{' {
		g.advance(1)
		g.pushFlowReturn(parseState{kind: parseStateFlowMappingCommaOrEnd})
		g.state.kind = parseStateFlowMappingKey
		return &YAMLEvent{Kind: YAMLEventStartObject}, nil, true
	}

	if len(slice) > 0 && slice[0] == '[' {
		g.advance(1)
		g.pushFlowReturn(parseState{kind: parseStateFlowMappingCommaOrEnd})
		g.state.kind = parseStateFlowSequenceEntry
		return &YAMLEvent{Kind: YAMLEventStartArray}, nil, true
	}

	val, style, ok := g.readFlowScalar(buf, base, isEnding)
	if !ok {
		return nil, nil, false
	}

	g.state.kind = parseStateFlowMappingCommaOrEnd
	return emitScalar(val, style), nil, true
}

func (g *YAMLEventGenerator) scanFlowMappingCommaOrEnd(buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if len(g.cur(buf, base)) == 0 {
		return nil, nil, false
	}

	c := g.cur(buf, base)[0]
	if c == ',' {
		g.advance(1)
		g.skipFlowWhitespace(buf, base)
		g.state.kind = parseStateFlowMappingKey
		return g.scanFlowMappingKey(buf, base, isEnding)
	}
	if c == '}' {
		g.advance(1)
		ret := g.popFlowReturn()
		g.state = ret
		return &YAMLEvent{Kind: YAMLEventEndObject}, nil, true
	}
	return nil, g.makeError("Expected ',' or '}' in flow mapping"), true
}

func (g *YAMLEventGenerator) scanFlowSequenceEntry(buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if len(g.cur(buf, base)) > 0 && g.cur(buf, base)[0] == ']' {
		g.advance(1)
		ret := g.popFlowReturn()
		g.state = ret
		return &YAMLEvent{Kind: YAMLEventEndArray}, nil, true
	}

	slice := g.cur(buf, base)
	if len(slice) > 0 && slice[0] == '{' {
		g.advance(1)
		g.pushFlowReturn(parseState{kind: parseStateFlowSequenceCommaOrEnd})
		g.state.kind = parseStateFlowMappingKey
		return &YAMLEvent{Kind: YAMLEventStartObject}, nil, true
	}

	if len(slice) > 0 && slice[0] == '[' {
		g.advance(1)
		g.pushFlowReturn(parseState{kind: parseStateFlowSequenceCommaOrEnd})
		g.state.kind = parseStateFlowSequenceEntry
		return &YAMLEvent{Kind: YAMLEventStartArray}, nil, true
	}

	val, style, ok := g.readFlowScalar(buf, base, isEnding)
	if !ok {
		return nil, nil, false
	}

	g.state.kind = parseStateFlowSequenceCommaOrEnd
	return emitScalar(val, style), nil, true
}

func (g *YAMLEventGenerator) scanFlowSequenceCommaOrEnd(buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if len(g.cur(buf, base)) == 0 {
		return nil, nil, false
	}

	c := g.cur(buf, base)[0]
	if c == ',' {
		g.advance(1)
		g.skipFlowWhitespace(buf, base)
		g.state.kind = parseStateFlowSequenceEntry
		return g.scanFlowSequenceEntry(buf, base, isEnding)
	}
	if c == ']' {
		g.advance(1)
		ret := g.popFlowReturn()
		g.state = ret
		return &YAMLEvent{Kind: YAMLEventEndArray}, nil, true
	}
	return nil, g.makeError("Expected ',' or ']' in flow sequence"), true
}

func (g *YAMLEventGenerator) readFlowScalar(buf []byte, base int64, isEnding bool) (string, YAMLScalarStyle, bool) {
	slice := g.cur(buf, base)
	if len(slice) == 0 {
		return "", 0, false
	}

	switch slice[0] {
	case '"':
		return g.readDoubleQuoted(buf, base)
	case '\'':
		return g.readSingleQuoted(buf, base)
	default:
		i := 0
		for i < len(slice) {
			c := slice[i]
			switch c {
			case ',', '}', ']':
				goto flowEnd
			case '#':
				if i > 0 && (slice[i-1] == ' ' || slice[i-1] == '\t') {
					goto flowEnd
				}
			case ':':
				if i+1 >= len(slice) || slice[i+1] == ' ' || slice[i+1] == '\t' || slice[i+1] == '\n' {
					goto flowEnd
				}
			case '\n', '\r':
				goto flowEnd
			}
			i++
		}
	flowEnd:
		raw := strings.TrimRight(string(slice[:i]), " \t")
		g.advance(i)
		return raw, YAMLScalarPlain, true
	}
}

func (g *YAMLEventGenerator) readDoubleQuoted(buf []byte, base int64) (string, YAMLScalarStyle, bool) {
	slice := g.cur(buf, base)
	if len(slice) == 0 || slice[0] != '"' {
		return "", 0, false
	}

	var result strings.Builder
	i := 1
	for {
		if i >= len(slice) {
			return "", 0, false
		}

		c := slice[i]
		switch c {
		case '"':
			i++
			g.advance(i)
			return result.String(), YAMLScalarDoubleQuoted, true
		case '\\':
			i++
			if i >= len(slice) {
				return "", 0, false
			}
			esc := slice[i]
			i++
			switch esc {
			case '"':
				result.WriteRune('"')
			case '\\':
				result.WriteRune('\\')
			case '/':
				result.WriteRune('/')
			case 'n':
				result.WriteRune('\n')
			case 'r':
				result.WriteRune('\r')
			case 't':
				result.WriteRune('\t')
			case 'b':
				result.WriteRune('')
			case 'f':
				result.WriteRune('')
			case 'u':
				if i+4 > len(slice) {
					return "", 0, false
				}
				hexStr := string(slice[i : i+4])
				i += 4
				var cp uint32
				fmt.Sscanf(hexStr, "%x", &cp)
				if cp == 0 {
					cp = 0xFFFD
				}
				result.WriteRune(rune(cp))
			case 'U':
				if i+8 > len(slice) {
					return "", 0, false
				}
				hexStr := string(slice[i : i+8])
				i += 8
				var cp uint32
				fmt.Sscanf(hexStr, "%x", &cp)
				if cp == 0 {
					cp = 0xFFFD
				}
				result.WriteRune(rune(cp))
			case '\n':
				for i < len(slice) && (slice[i] == ' ' || slice[i] == '\t') {
					i++
				}
			default:
				result.WriteRune(rune(esc))
			}
		case '\n':
			result.WriteRune(' ')
			i++
		default:
			result.WriteRune(rune(c))
			i++
		}
	}
}

func (g *YAMLEventGenerator) readSingleQuoted(buf []byte, base int64) (string, YAMLScalarStyle, bool) {
	slice := g.cur(buf, base)
	if len(slice) == 0 || slice[0] != '\'' {
		return "", 0, false
	}

	var result strings.Builder
	i := 1
	for {
		if i >= len(slice) {
			return "", 0, false
		}

		c := slice[i]
		switch c {
		case '\'':
			i++
			if i < len(slice) && slice[i] == '\'' {
				result.WriteRune('\'')
				i++
			} else {
				g.advance(i)
				return result.String(), YAMLScalarSingleQuoted, true
			}
		case '\n':
			result.WriteRune(' ')
			i++
		default:
			result.WriteRune(rune(c))
			i++
		}
	}
}

func (g *YAMLEventGenerator) scanScalarAsValue(buf []byte, base int64, isEnding bool, isKey bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	slice := g.cur(buf, base)

	if style, chomp := isBlockScalarIndicator(slice); style != -1 {
		g.advance(1)
		g.consumeBlockScalarHeader(buf, base, isEnding)
		g.blockScalarBuf.Reset()
		g.state.kind = parseStateBlockScalarContent
		g.state.scalarStyle = style
		g.state.contentIndent = MaxIndentSentinel
		g.state.chomp = chomp
		return g.scanBlockScalarContent(buf, base, isEnding, style, MaxIndentSentinel, chomp)
	}

	if len(slice) > 0 && slice[0] == '"' {
		val, _, ok := g.readDoubleQuoted(buf, base)
		if !ok {
			return nil, nil, false
		}
		g.restoreBlockStateAfterValue()
		return &YAMLEvent{Kind: YAMLEventString, String: val}, nil, true
	}

	if len(slice) > 0 && slice[0] == '\'' {
		val, _, ok := g.readSingleQuoted(buf, base)
		if !ok {
			return nil, nil, false
		}
		g.restoreBlockStateAfterValue()
		return &YAMLEvent{Kind: YAMLEventString, String: val}, nil, true
	}

	lineEnd := findNewline(slice)
	if lineEnd < 0 {
		if isEnding {
			lineEnd = len(slice)
		} else {
			return nil, nil, false
		}
	}

	raw := trimPlainScalar(slice[:lineEnd])
	g.consumeBytesAndNewline(buf, base, lineEnd)
	g.restoreBlockStateAfterValue()

	return emitScalar(raw, YAMLScalarPlain), nil, true
}

func (g *YAMLEventGenerator) restoreBlockStateAfterValue() {
	if len(g.indentStack) > 0 {
		e := g.indentStack[len(g.indentStack)-1]
		if e.context == blockContextMapping {
			g.state.kind = parseStateBlockMappingKey
			g.state.indent = e.indent
		} else {
			g.state.kind = parseStateBlockSequenceEntry
			g.state.indent = e.indent
		}
	} else {
		g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventDocumentEnd})
		g.state.kind = parseStateBeforeDocument
	}
}

func (g *YAMLEventGenerator) blockReturnStateAfterValue() parseState {
	if len(g.indentStack) > 0 {
		e := g.indentStack[len(g.indentStack)-1]
		if e.context == blockContextMapping {
			return parseState{kind: parseStateBlockMappingKey, indent: e.indent}
		}
		return parseState{kind: parseStateBlockSequenceEntry, indent: e.indent}
	}
	return parseState{kind: parseStateBeforeDocument}
}

func (g *YAMLEventGenerator) pushFlowReturn(ret parseState) {
	g.flowReturnStates = append(g.flowReturnStates, ret)
}

func (g *YAMLEventGenerator) popFlowReturn() parseState {
	if len(g.flowReturnStates) > 0 {
		ret := g.flowReturnStates[len(g.flowReturnStates)-1]
		g.flowReturnStates = g.flowReturnStates[:len(g.flowReturnStates)-1]
		return ret
	}
	return parseState{kind: parseStateBeforeDocument}
}

func (g *YAMLEventGenerator) closeAllThenEof(isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if !isEnding {
		return nil, nil, false
	}

	for len(g.indentStack) > 0 {
		e := g.indentStack[len(g.indentStack)-1]
		g.indentStack = g.indentStack[:len(g.indentStack)-1]
		if e.context == blockContextMapping {
			g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventEndObject})
		} else {
			g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventEndArray})
		}
	}

	g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventDocumentEnd})
	g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventStreamEnd})
	g.state.kind = parseStateDone
	return &YAMLEvent{Kind: YAMLEventEof}, nil, true
}

func (g *YAMLEventGenerator) closeBlocksToIndent(targetIndent int, buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	var first *YAMLEvent
	for len(g.indentStack) > 0 && g.indentStack[len(g.indentStack)-1].indent > targetIndent {
		e := g.indentStack[len(g.indentStack)-1]
		g.indentStack = g.indentStack[:len(g.indentStack)-1]
		if e.context == blockContextMapping {
			first = &YAMLEvent{Kind: YAMLEventEndObject}
		} else {
			first = &YAMLEvent{Kind: YAMLEventEndArray}
		}

		if len(g.indentStack) == 0 || g.indentStack[len(g.indentStack)-1].indent <= targetIndent {
			break
		}
		g.pendingEvents = append(g.pendingEvents, first)
	}

	if first == nil {
		return g.scan(buf, base, isEnding)
	}

	if len(g.indentStack) > 0 {
		e := g.indentStack[len(g.indentStack)-1]
		if e.context == blockContextMapping {
			g.state.kind = parseStateBlockMappingKey
			g.state.indent = e.indent
		} else {
			g.state.kind = parseStateBlockSequenceEntry
			g.state.indent = e.indent
		}
	} else {
		slice := g.cur(buf, base)
		if len(slice) == 0 {
			g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventDocumentEnd})
			g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventStreamEnd})
			g.state.kind = parseStateDone
		} else {
			g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventDocumentEnd})
			g.state.kind = parseStateBeforeDocument
		}
	}

	return first, nil, true
}

func (g *YAMLEventGenerator) closeBlocksForDocMarker(buf []byte, base int64, isEnding bool) (*YAMLEvent, *YAMLSyntaxError, bool) {
	if len(g.indentStack) > 0 {
		e := g.indentStack[len(g.indentStack)-1]
		g.indentStack = g.indentStack[:len(g.indentStack)-1]

		var first *YAMLEvent
		if e.context == blockContextMapping {
			first = &YAMLEvent{Kind: YAMLEventEndObject}
		} else {
			first = &YAMLEvent{Kind: YAMLEventEndArray}
		}

		for len(g.indentStack) > 0 {
			e := g.indentStack[len(g.indentStack)-1]
			g.indentStack = g.indentStack[:len(g.indentStack)-1]
			if e.context == blockContextMapping {
				g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventEndObject})
			} else {
				g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventEndArray})
			}
		}

		g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventDocumentEnd})
		g.state.kind = parseStateBeforeDocument
		return first, nil, true
	}

	g.pendingEvents = append(g.pendingEvents, &YAMLEvent{Kind: YAMLEventDocumentEnd})
	g.state.kind = parseStateBeforeDocument
	return g.scan(buf, base, isEnding)
}

func (g *YAMLEventGenerator) cur(buf []byte, base int64) []byte {
	offset := int(g.fileOffset - base)
	if offset < 0 {
		offset = 0
	}
	if offset > len(buf) {
		offset = len(buf)
	}
	return buf[offset:]
}

func (g *YAMLEventGenerator) advance(n int) {
	g.fileOffset += int64(n)
}

func (g *YAMLEventGenerator) consumeBytesAndNewline(buf []byte, base int64, n int) {
	g.advance(n)
	slice := g.cur(buf, base)
	if len(slice) > 0 && slice[0] == '\r' {
		g.advance(1)
		if len(g.cur(buf, base)) > 0 && g.cur(buf, base)[0] == '\n' {
			g.advance(1)
		}
	} else if len(slice) > 0 && slice[0] == '\n' {
		g.advance(1)
	}
	g.fileLine++
	g.fileStartOfLastLine = g.fileOffset
}

func (g *YAMLEventGenerator) advanceToNextLine(buf []byte, base int64, isEnding bool) {
	slice := g.cur(buf, base)
	n := findNewline(slice)
	if n < 0 {
		if isEnding {
			n = len(slice)
		} else {
			return
		}
	}
	if n > 0 || isEnding {
		g.consumeBytesAndNewline(buf, base, n)
	}
}

func (g *YAMLEventGenerator) consumeLine(buf []byte, base int64, isEnding bool) {
	g.advanceToNextLine(buf, base, isEnding)
}

func (g *YAMLEventGenerator) consumeBlockScalarHeader(buf []byte, base int64, isEnding bool) {
	g.advanceToNextLine(buf, base, isEnding)
}

func (g *YAMLEventGenerator) skipBlockWhitespace(buf []byte, base int64, isEnding bool) bool {
	for {
		slice := g.cur(buf, base)
		if len(slice) == 0 {
			return true
		}

		lineEnd := findNewline(slice)
		if lineEnd < 0 {
			if isEnding {
				lineEnd = len(slice)
			} else {
				allBlank := true
				for _, b := range slice {
					if b != ' ' && b != '\t' {
						allBlank = false
						break
					}
				}
				return allBlank
			}
		}

		line := slice[:lineEnd]
		nonSpace := -1
		for i, b := range line {
			if b != ' ' && b != '\t' {
				nonSpace = i
				break
			}
		}

		if nonSpace == -1 {
			g.consumeBytesAndNewline(buf, base, lineEnd)
		} else if line[nonSpace] == '#' {
			g.consumeBytesAndNewline(buf, base, lineEnd)
		} else {
			return true
		}
	}
}

func (g *YAMLEventGenerator) skipFlowWhitespace(buf []byte, base int64) {
	slice := g.cur(buf, base)
	for _, b := range slice {
		switch b {
		case ' ', '\t':
			g.advance(1)
		case '\n', '\r':
			g.advance(1)
			g.fileLine++
			g.fileStartOfLastLine = g.fileOffset
		default:
			return
		}
	}
}

func (g *YAMLEventGenerator) peekDocMarker(slice []byte, marker string, isEnding bool) bool {
	if len(slice) < len(marker) {
		return false
	}
	if string(slice[:len(marker)]) != marker {
		return false
	}
	if len(slice) == len(marker) {
		return isEnding
	}
	next := slice[len(marker)]
	return next == ' ' || next == '\t' || next == '\n' || next == '\r' || next == '#'
}

func (g *YAMLEventGenerator) makeError(msg string) *YAMLSyntaxError {
	col := g.fileOffset - g.fileStartOfLastLine
	return &YAMLSyntaxError{
		Location: TextPosition{
			Line:   g.fileLine,
			Column: col,
			Offset: g.fileOffset,
		},
		Message: msg,
	}
}

// Helper functions

func measureIndent(line []byte) int {
	count := 0
	for _, b := range line {
		if b == ' ' || b == '\t' {
			count++
		} else {
			break
		}
	}
	return count
}

func findNewline(slice []byte) int {
	for i, b := range slice {
		if b == '\n' || b == '\r' {
			return i
		}
	}
	return -1
}

func isSeqEntry(content []byte) bool {
	if len(content) == 0 || content[0] != '-' {
		return false
	}
	if len(content) == 1 {
		return true
	}
	switch content[1] {
	case ' ', '\t', '\n', '\r':
		return true
	default:
		return false
	}
}

func findBlockMappingColon(content []byte) int {
	var inSingle, inDouble bool
	for i := 0; i < len(content); i++ {
		if inSingle {
			if content[i] == '\'' {
				if i+1 < len(content) && content[i+1] == '\'' {
					i++
				} else {
					inSingle = false
				}
			}
			continue
		}
		if inDouble {
			if content[i] == '\\' {
				i++
				continue
			}
			if content[i] == '"' {
				inDouble = false
			}
			continue
		}

		switch content[i] {
		case '\'':
			inSingle = true
		case '"':
			inDouble = true
		case '#':
			return -1
		case ':':
			if i+1 >= len(content) || content[i+1] == ' ' || content[i+1] == '\t' || content[i+1] == '\n' || content[i+1] == '\r' || content[i+1] == '#' {
				return i
			}
		}
	}
	return -1
}

func isBlockScalarIndicator(content []byte) (YAMLScalarStyle, chompKind) {
	if len(content) == 0 {
		return -1, chompClip
	}

	var style YAMLScalarStyle
	switch content[0] {
	case '|':
		style = YAMLScalarLiteral
	case '>':
		style = YAMLScalarFolded
	default:
		return -1, chompClip
	}

	chomp := chompClip
	if len(content) > 1 {
		switch content[1] {
		case '-':
			chomp = chompStrip
		case '+':
			chomp = chompKeep
		}
	}

	return style, chomp
}

func trimPlainScalar(line []byte) string {
	end := len(line)
	for i := 0; i < len(line); i++ {
		if line[i] == '#' && i > 0 && (line[i-1] == ' ' || line[i-1] == '\t') {
			end = i
			break
		}
	}
	return strings.TrimRight(string(line[:end]), " \t")
}

func applyChomp(s string, chomp chompKind) string {
	switch chomp {
	case chompStrip:
		return strings.TrimRight(s, "\n")
	case chompClip:
		return strings.TrimRight(s, "\n")
	case chompKeep:
		return s
	}
	return s
}

func emitScalar(val string, style YAMLScalarStyle) *YAMLEvent {
	switch style {
	case YAMLScalarSingleQuoted, YAMLScalarDoubleQuoted, YAMLScalarLiteral, YAMLScalarFolded:
		return &YAMLEvent{Kind: YAMLEventString, String: val}
	case YAMLScalarPlain:
		switch val {
		case "true", "True", "TRUE":
			return &YAMLEvent{Kind: YAMLEventBoolean, Boolean: true}
		case "false", "False", "FALSE":
			return &YAMLEvent{Kind: YAMLEventBoolean, Boolean: false}
		case "null", "Null", "NULL", "~", "":
			return &YAMLEvent{Kind: YAMLEventNull}
		default:
			if isYAMLNumber(val) {
				return &YAMLEvent{Kind: YAMLEventNumber, String: val}
			}
			return &YAMLEvent{Kind: YAMLEventString, String: val}
		}
	}
	return &YAMLEvent{Kind: YAMLEventString, String: val}
}

func isYAMLNumber(s string) bool {
	s = strings.TrimSpace(s)
	if len(s) == 0 {
		return false
	}

	special := []string{".inf", ".Inf", ".INF", "-.inf", "-.Inf", "-.INF", "+.inf", "+.Inf", "+.INF",
		".nan", ".NaN", ".NAN"}
	for _, sp := range special {
		if s == sp {
			return true
		}
	}

	bytes := []byte(s)
	start := 0
	if len(bytes) > 0 && (bytes[0] == '+' || bytes[0] == '-') {
		start = 1
	}

	if start >= len(bytes) {
		return false
	}

	rest := bytes[start:]

	if len(rest) >= 2 && ((rest[0] == '0' && rest[1] == 'x') || (rest[0] == '0' && rest[1] == 'X')) {
		for _, b := range rest[2:] {
			if !((b >= '0' && b <= '9') || (b >= 'a' && b <= 'f') || (b >= 'A' && b <= 'F')) {
				return false
			}
		}
		return true
	}

	if len(rest) >= 2 && ((rest[0] == '0' && rest[1] == 'o') || (rest[0] == '0' && rest[1] == 'O')) {
		for _, b := range rest[2:] {
			if b < '0' || b > '7' {
				return false
			}
		}
		return true
	}

	if len(rest) >= 2 && ((rest[0] == '0' && rest[1] == 'b') || (rest[0] == '0' && rest[1] == 'B')) {
		for _, b := range rest[2:] {
			if b != '0' && b != '1' {
				return false
			}
		}
		return true
	}

	i := 0
	for i < len(rest) && rest[i] >= '0' && rest[i] <= '9' {
		i++
	}

	if i == 0 {
		return false
	}

	if i == len(rest) {
		return true
	}

	if rest[i] == '.' {
		i++
		for i < len(rest) && rest[i] >= '0' && rest[i] <= '9' {
			i++
		}
	}

	if i == len(rest) {
		return true
	}

	if rest[i] == 'e' || rest[i] == 'E' {
		i++
		if i < len(rest) && (rest[i] == '+' || rest[i] == '-') {
			i++
		}
		expStart := i
		for i < len(rest) && rest[i] >= '0' && rest[i] <= '9' {
			i++
		}
		if i == expStart {
			return false
		}
	}

	return i == len(rest)
}
