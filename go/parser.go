package parser

import (
	"fmt"
	"strings"
	"unicode/utf8"
)

const MaxStateStackSize = 65_536

// JSONEventKind represents the kind of JSON event
type JSONEventKind int

const (
	EventString JSONEventKind = iota
	EventNumber
	EventBoolean
	EventNull
	EventStartArray
	EventEndArray
	EventStartObject
	EventEndObject
	EventObjectKey
	EventEof
)

// JSONEvent represents a parsed JSON event
type JSONEvent struct {
	Kind    JSONEventKind
	String  string
	Boolean bool
}

// TextPosition represents a location in parsed text
type TextPosition struct {
	Line   int64
	Column int64
	Offset int64
}

// JSONSyntaxError represents a syntax error in JSON
type JSONSyntaxError struct {
	Location struct {
		Start TextPosition
		End   TextPosition
	}
	Message string
}

// Error implements the error interface
func (e *JSONSyntaxError) Error() string {
	startLine := e.Location.Start.Line + 1
	startCol := e.Location.Start.Column + 1
	endLine := e.Location.End.Line + 1
	endCol := e.Location.End.Column + 1

	if e.Location.Start.Offset+1 >= e.Location.End.Offset {
		return fmt.Sprintf("Parser error at line %d column %d: %s", startLine, startCol, e.Message)
	} else if e.Location.Start.Line == e.Location.End.Line {
		return fmt.Sprintf("Parser error at line %d between columns %d and column %d: %s",
			startLine, startCol, endCol, e.Message)
	}
	return fmt.Sprintf("Parser error between line %d column %d and line %d column %d: %s",
		startLine, startCol, endLine, endCol, e.Message)
}

// JSONEventWrapper wraps a JSON event with metadata
type JSONEventWrapper struct {
	ConsumedBytes int
	Event         *JSONEvent
	Err           error
}

// JSONStateKind represents the parser state
type JSONStateKind int

const (
	StateObjectKey JSONStateKind = iota
	StateObjectKeyOrEnd
	StateObjectColon
	StateObjectValue
	StateObjectCommaOrEnd
	StateArrayValue
	StateArrayValueOrEnd
	StateArrayCommaOrEnd
)

// JSONTokenKind represents the kind of token
type JSONTokenKind int

const (
	TokenOpeningSquareBracket JSONTokenKind = iota
	TokenClosingSquareBracket
	TokenOpeningCurlyBracket
	TokenClosingCurlyBracket
	TokenComma
	TokenColon
	TokenString
	TokenNumber
	TokenTrue
	TokenFalse
	TokenNull
	TokenEof
)

// JSONToken represents a parsed JSON token
type JSONToken struct {
	Kind   JSONTokenKind
	String string
}

// JSONEventGenerator is the main parser
type JSONEventGenerator struct {
	lexer              *JSONLexer
	stateStack         []JSONStateKind
	maxStateStackSize  int
	elementRead        bool
	bufferedEvent      *JSONEvent
	bufferedEventError error
}

// NewJSONEventGenerator creates a new parser
func NewJSONEventGenerator() *JSONEventGenerator {
	return &JSONEventGenerator{
		lexer: &JSONLexer{
			fileOffset:           0,
			fileLine:             0,
			fileStartOfLastLine:  0,
			fileStartOfLastToken: 0,
			isStart:              true,
		},
		stateStack:         make([]JSONStateKind, 0),
		maxStateStackSize:  MaxStateStackSize,
		elementRead:        false,
		bufferedEvent:      nil,
		bufferedEventError: nil,
	}
}

// WithMaxStackSize sets the maximum stack size
func (g *JSONEventGenerator) WithMaxStackSize(size int) *JSONEventGenerator {
	g.maxStateStackSize = size
	return g
}

// NextEvent reads the next JSON event from input buffer
func (g *JSONEventGenerator) NextEvent(inputBuffer []byte, isEnding bool) JSONEventWrapper {
	if g.bufferedEvent != nil || g.bufferedEventError != nil {
		event := g.bufferedEvent
		err := g.bufferedEventError
		g.bufferedEvent = nil
		g.bufferedEventError = nil
		return JSONEventWrapper{
			ConsumedBytes: 0,
			Event:         event,
			Err:           err,
		}
	}

	startFileOffset := g.lexer.fileOffset
	for {
		sliceStart := int(g.lexer.fileOffset - startFileOffset)
		if sliceStart > len(inputBuffer) {
			sliceStart = len(inputBuffer)
		}
		if sliceStart < 0 {
			sliceStart = 0
		}

		token, err, ok := g.lexer.readNextToken(inputBuffer[sliceStart:], isEnding)

		consumedBytes := int(g.lexer.fileOffset - startFileOffset)
		if consumedBytes < 0 {
			consumedBytes = 0
		}

		if err != nil {
			return JSONEventWrapper{
				ConsumedBytes: consumedBytes,
				Err:           err,
			}
		}

		if token != nil {
			event, tokenErr := g.applyNewToken(*token)
			if tokenErr != nil {
				syntaxErr := g.lexer.syntaxError(
					g.lexer.fileStartOfLastToken,
					g.lexer.fileOffset,
					tokenErr.Error(),
				)
				return JSONEventWrapper{
					ConsumedBytes: consumedBytes,
					Err:           syntaxErr,
				}
			}

			if event != nil {
				return JSONEventWrapper{
					ConsumedBytes: consumedBytes,
					Event:         event,
					Err:           nil,
				}
			}
		}

		if !ok {
			break
		}
	}

	consumedBytes := int(g.lexer.fileOffset - startFileOffset)
	if isEnding {
		g.bufferedEvent = &JSONEvent{Kind: EventEof}
		syntaxErr := g.lexer.syntaxError(
			g.lexer.fileOffset,
			g.lexer.fileOffset+1,
			"Unexpected end of file",
		)
		return JSONEventWrapper{
			ConsumedBytes: consumedBytes,
			Err:           syntaxErr,
		}
	}

	return JSONEventWrapper{
		ConsumedBytes: consumedBytes,
		Event:         nil,
		Err:           nil,
	}
}

func (g *JSONEventGenerator) applyNewToken(token JSONToken) (*JSONEvent, error) {
	if len(g.stateStack) == 0 {
		if g.elementRead {
			if token.Kind == TokenEof {
				return &JSONEvent{Kind: EventEof}, nil
			}
			return nil, fmt.Errorf("The JSON already contains one root element")
		}
		g.elementRead = true
		return g.applyNewTokenForValue(token)
	}

	state := g.stateStack[len(g.stateStack)-1]
	g.stateStack = g.stateStack[:len(g.stateStack)-1]

	switch state {
	case StateObjectKeyOrEnd:
		if token.Kind == TokenClosingCurlyBracket {
			return &JSONEvent{Kind: EventEndObject}, nil
		}
		if err := g.pushStateStack(StateObjectKey); err != nil {
			return nil, err
		}
		return g.applyNewToken(token)

	case StateObjectKey:
		if token.Kind == TokenClosingCurlyBracket {
			return nil, fmt.Errorf("Trailing commas are not allowed")
		}
		if err := g.pushStateStack(StateObjectColon); err != nil {
			return nil, err
		}
		if token.Kind == TokenString {
			return &JSONEvent{Kind: EventObjectKey, String: token.String}, nil
		}
		return nil, fmt.Errorf("Object keys must be strings")

	case StateObjectColon:
		if err := g.pushStateStack(StateObjectValue); err != nil {
			return nil, err
		}
		if token.Kind == TokenColon {
			return nil, nil
		}
		event, _ := g.applyNewToken(token)
		if event != nil {
			g.bufferedEvent = event
		}
		return nil, fmt.Errorf("Object keys must be followed by a colon ':'")

	case StateObjectValue:
		if err := g.pushStateStack(StateObjectCommaOrEnd); err != nil {
			return nil, err
		}
		return g.applyNewTokenForValue(token)

	case StateObjectCommaOrEnd:
		switch token.Kind {
		case TokenComma:
			return nil, g.pushStateStack(StateObjectKey)
		case TokenClosingCurlyBracket:
			return &JSONEvent{Kind: EventEndObject}, nil
		default:
			return nil, fmt.Errorf("Object values must be followed by a comma to add a new value or a curly bracket to end the object")
		}

	case StateArrayValueOrEnd:
		if token.Kind == TokenClosingSquareBracket {
			return &JSONEvent{Kind: EventEndArray}, nil
		}
		if err := g.pushStateStack(StateArrayValue); err != nil {
			return nil, err
		}
		return g.applyNewToken(token)

	case StateArrayValue:
		if token.Kind == TokenClosingSquareBracket {
			return nil, fmt.Errorf("Trailing commas are not allowed")
		}
		if err := g.pushStateStack(StateArrayCommaOrEnd); err != nil {
			return nil, err
		}
		return g.applyNewTokenForValue(token)

	case StateArrayCommaOrEnd:
		switch token.Kind {
		case TokenComma:
			return nil, g.pushStateStack(StateArrayValue)
		case TokenClosingSquareBracket:
			return &JSONEvent{Kind: EventEndArray}, nil
		default:
			_ = g.pushStateStack(StateArrayValue)
			event, _ := g.applyNewToken(token)
			if event != nil {
				g.bufferedEvent = event
			}
			return nil, fmt.Errorf("Array values must be followed by a comma to add a new value or a squared bracket to end the array")
		}
	}

	return nil, nil
}

func (g *JSONEventGenerator) applyNewTokenForValue(token JSONToken) (*JSONEvent, error) {
	switch token.Kind {
	case TokenOpeningSquareBracket:
		return &JSONEvent{Kind: EventStartArray}, g.pushStateStack(StateArrayValueOrEnd)

	case TokenClosingSquareBracket:
		return nil, fmt.Errorf("Unexpected closing square bracket, no array to close")

	case TokenOpeningCurlyBracket:
		return &JSONEvent{Kind: EventStartObject}, g.pushStateStack(StateObjectKeyOrEnd)

	case TokenClosingCurlyBracket:
		return nil, fmt.Errorf("Unexpected closing curly bracket, no object to close")

	case TokenComma:
		return nil, fmt.Errorf("Unexpected comma, no values to separate")

	case TokenColon:
		return nil, fmt.Errorf("Unexpected colon, no key to follow")

	case TokenString:
		return &JSONEvent{Kind: EventString, String: token.String}, nil

	case TokenNumber:
		return &JSONEvent{Kind: EventNumber, String: token.String}, nil

	case TokenTrue:
		return &JSONEvent{Kind: EventBoolean, Boolean: true}, nil

	case TokenFalse:
		return &JSONEvent{Kind: EventBoolean, Boolean: false}, nil

	case TokenNull:
		return &JSONEvent{Kind: EventNull}, nil

	case TokenEof:
		return &JSONEvent{Kind: EventEof}, fmt.Errorf("Unexpected end of file, a value was expected")
	}

	return nil, nil
}

func (g *JSONEventGenerator) pushStateStack(state JSONStateKind) error {
	if err := g.checkStackSize(); err != nil {
		return err
	}
	g.stateStack = append(g.stateStack, state)
	return nil
}

func (g *JSONEventGenerator) checkStackSize() error {
	if len(g.stateStack) > g.maxStateStackSize {
		return fmt.Errorf("Max stack size of %d reached on an object opening", g.maxStateStackSize)
	}
	return nil
}

// JSONLexer is the lexical analyzer
type JSONLexer struct {
	fileOffset           int64
	fileLine             int64
	fileStartOfLastLine  int64
	fileStartOfLastToken int64
	isStart              bool
}

func (l *JSONLexer) readNextToken(inputBuffer []byte, isEnding bool) (*JSONToken, *JSONSyntaxError, bool) {
	// Remove BOM at the beginning
	if l.isStart {
		if len(inputBuffer) < 3 && !isEnding {
			return nil, nil, false
		}
		l.isStart = false
		if len(inputBuffer) >= 3 && inputBuffer[0] == 0xEF && inputBuffer[1] == 0xBB && inputBuffer[2] == 0xBF {
			inputBuffer = inputBuffer[3:]
			l.fileOffset += 3
		}
	}

	// Skip whitespaces
	i := 0
	for i < len(inputBuffer) {
		c := inputBuffer[i]
		switch c {
		case ' ', '\t':
			i++
		case '\n':
			i++
			l.fileLine++
			l.fileStartOfLastLine = l.fileOffset + int64(i)
		case '\r':
			i++
			if i < len(inputBuffer) && inputBuffer[i] == '\n' {
				i++
			} else if !isEnding {
				i--
				l.fileOffset += int64(i)
				return nil, nil, false
			}
			l.fileLine++
			l.fileStartOfLastLine = l.fileOffset + int64(i)
		default:
			goto skipDone
		}
	}

skipDone:
	l.fileOffset += int64(i)
	inputBuffer = inputBuffer[i:]
	l.fileStartOfLastToken = l.fileOffset

	if isEnding && len(inputBuffer) == 0 {
		return &JSONToken{Kind: TokenEof}, nil, true
	}

	if len(inputBuffer) == 0 {
		return nil, nil, false
	}

	c := inputBuffer[0]
	switch c {
	case '{':
		l.fileOffset++
		return &JSONToken{Kind: TokenOpeningCurlyBracket}, nil, true
	case '}':
		l.fileOffset++
		return &JSONToken{Kind: TokenClosingCurlyBracket}, nil, true
	case '[':
		l.fileOffset++
		return &JSONToken{Kind: TokenOpeningSquareBracket}, nil, true
	case ']':
		l.fileOffset++
		return &JSONToken{Kind: TokenClosingSquareBracket}, nil, true
	case ',':
		l.fileOffset++
		return &JSONToken{Kind: TokenComma}, nil, true
	case ':':
		l.fileOffset++
		return &JSONToken{Kind: TokenColon}, nil, true
	case '"':
		return l.readString(inputBuffer)
	case 't':
		return l.readConstant(inputBuffer, isEnding, "true", TokenTrue)
	case 'f':
		return l.readConstant(inputBuffer, isEnding, "false", TokenFalse)
	case 'n':
		return l.readConstant(inputBuffer, isEnding, "null", TokenNull)
	case '-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9':
		return l.readNumber(inputBuffer, isEnding)
	default:
		l.fileOffset++
		var msg string
		if c < 128 {
			msg = fmt.Sprintf("Unexpected char: '%c'", c)
		} else {
			msg = fmt.Sprintf("Unexpected byte: \\x%X", c)
		}
		err := l.syntaxError(l.fileOffset-1, l.fileOffset, msg)
		return nil, err, true
	}
}

func (l *JSONLexer) readString(inputBuffer []byte) (*JSONToken, *JSONSyntaxError, bool) {
	var parseError *JSONSyntaxError
	var resultString strings.Builder
	var hasEscapes bool
	nextByteOffset := 1

	for nextByteOffset < len(inputBuffer) {
		c := inputBuffer[nextByteOffset]
		switch c {
		case '"':
			// End of string
			if !hasEscapes {
				str, err := l.decodeUTF8(inputBuffer[1:nextByteOffset], l.fileOffset+1)
				l.fileOffset += int64(nextByteOffset) + 1
				return &JSONToken{Kind: TokenString, String: str}, err, true
			}
			l.fileOffset += int64(nextByteOffset) + 1
			return &JSONToken{Kind: TokenString, String: resultString.String()}, parseError, true

		case '\\':
			// Escape sequences
			if !hasEscapes {
				resultString.Write(inputBuffer[1:nextByteOffset])
				hasEscapes = true
			}
			nextByteOffset++
			if nextByteOffset >= len(inputBuffer) {
				return nil, nil, false
			}
			escaped := inputBuffer[nextByteOffset]
			switch escaped {
			case '"':
				resultString.WriteRune('"')
				nextByteOffset++
			case '\\':
				resultString.WriteRune('\\')
				nextByteOffset++
			case '/':
				resultString.WriteRune('/')
				nextByteOffset++
			case 'b':
				resultString.WriteRune('\b')
				nextByteOffset++
			case 'f':
				resultString.WriteRune('\f')
				nextByteOffset++
			case 'n':
				resultString.WriteRune('\n')
				nextByteOffset++
			case 'r':
				resultString.WriteRune('\r')
				nextByteOffset++
			case 't':
				resultString.WriteRune('\t')
				nextByteOffset++
			case 'u':
				nextByteOffset++
				if nextByteOffset+4 > len(inputBuffer) {
					return nil, nil, false
				}
				val := inputBuffer[nextByteOffset : nextByteOffset+4]
				nextByteOffset += 4
				codePoint, err := readHexaChar(val)
				if err != nil {
					if parseError == nil {
						parseError = l.syntaxError(
							l.fileOffset+int64(nextByteOffset)-4,
							l.fileOffset+int64(nextByteOffset),
							err.Error(),
						)
					}
					resultString.WriteRune(utf8.RuneError)
				} else if codePoint >= 0xD800 && codePoint <= 0xDBFF {
					// High surrogate
					highSurrogate := codePoint
					if nextByteOffset+6 > len(inputBuffer) {
						return nil, nil, false
					}
					val2 := inputBuffer[nextByteOffset : nextByteOffset+6]
					if len(val2) < 2 || val2[0] != '\\' || val2[1] != 'u' {
						if parseError == nil {
							parseError = l.syntaxError(
								l.fileOffset+int64(nextByteOffset),
								l.fileOffset+int64(nextByteOffset)+6,
								fmt.Sprintf("\\u%X is a high surrogate and should be followed by a low surrogate \\uXXXX", highSurrogate),
							)
						}
						nextByteOffset += 6
					} else {
						nextByteOffset += 6
						lowSurrogate, err := readHexaChar(val2[2:])
						if err != nil {
							if parseError == nil {
								parseError = l.syntaxError(
									l.fileOffset+int64(nextByteOffset)-6,
									l.fileOffset+int64(nextByteOffset),
									err.Error(),
								)
							}
							resultString.WriteRune(utf8.RuneError)
						} else if lowSurrogate < 0xDC00 || lowSurrogate > 0xDFFF {
							if parseError == nil {
								parseError = l.syntaxError(
									l.fileOffset+int64(nextByteOffset)-6,
									l.fileOffset+int64(nextByteOffset),
									fmt.Sprintf("\\u%X is not a valid low surrogate", lowSurrogate),
								)
							}
						} else {
							codePoint := 0x10000 + ((highSurrogate & 0x03FF) << 10) + (lowSurrogate & 0x03FF)
							if r := rune(codePoint); utf8.ValidRune(r) {
								resultString.WriteRune(r)
							} else {
								resultString.WriteRune(utf8.RuneError)
								if parseError == nil {
									parseError = l.syntaxError(
										l.fileOffset+int64(nextByteOffset)-12,
										l.fileOffset+int64(nextByteOffset),
										fmt.Sprintf("\\u%X\\u%X is an invalid surrogate pair", highSurrogate, lowSurrogate),
									)
								}
							}
						}
					}
				} else if codePoint <= 0xDFFF {
					// Standalone low surrogate (invalid)
					if parseError == nil {
						parseError = l.syntaxError(
							l.fileOffset+int64(nextByteOffset)-4,
							l.fileOffset+int64(nextByteOffset),
							fmt.Sprintf("\\u%X is not a valid high surrogate", codePoint),
						)
					}
					resultString.WriteRune(utf8.RuneError)
				} else if r := rune(codePoint); utf8.ValidRune(r) {
					resultString.WriteRune(r)
				} else {
					resultString.WriteRune(utf8.RuneError)
				}

			default:
				nextByteOffset++
				if parseError == nil {
					parseError = l.syntaxError(
						l.fileOffset+int64(nextByteOffset)-2,
						l.fileOffset+int64(nextByteOffset),
						fmt.Sprintf("'\\%c' is not a valid escape sequence", escaped),
					)
				}
				resultString.WriteRune(utf8.RuneError)
			}

		case 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31:
			if parseError == nil {
				parseError = l.syntaxError(
					l.fileOffset+int64(nextByteOffset),
					l.fileOffset+int64(nextByteOffset)+1,
					fmt.Sprintf("'%c' is not allowed in JSON strings", c),
				)
			}
			nextByteOffset++

		default:
			// Only accumulate into resultString when hasEscapes is true.
			// When hasEscapes is false the raw bytes are decoded at the end via
			// decodeUTF8(inputBuffer[1:nextByteOffset]), so writing here would
			// duplicate bytes if an escape is seen later and triggers the
			// resultString.Write(inputBuffer[1:nextByteOffset]) catch-up.
			if hasEscapes {
				resultString.WriteByte(c)
			}
			nextByteOffset++
		}
	}

	return nil, nil, false
}

func (l *JSONLexer) readConstant(
	inputBuffer []byte,
	isEnding bool,
	expected string,
	value JSONTokenKind,
) (*JSONToken, *JSONSyntaxError, bool) {
	if len(inputBuffer) >= len(expected) && string(inputBuffer[:len(expected)]) == expected {
		l.fileOffset += int64(len(expected))
		return &JSONToken{Kind: value}, nil, true
	}

	asciiChars := 0
	for i := 0; i < len(inputBuffer); i++ {
		c := inputBuffer[i]
		if (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') {
			asciiChars++
		} else {
			break
		}
	}

	if asciiChars == len(inputBuffer) && !isEnding {
		return nil, nil, false
	}

	read := 1
	if asciiChars > read {
		read = asciiChars
	}
	startOffset := l.fileOffset
	l.fileOffset += int64(read)
	err := l.syntaxError(startOffset, l.fileOffset, fmt.Sprintf("%s expected", expected))
	return nil, err, true
}

func (l *JSONLexer) readNumber(inputBuffer []byte, isEnding bool) (*JSONToken, *JSONSyntaxError, bool) {
	nextByteOffset := 0

	if nextByteOffset >= len(inputBuffer) {
		return nil, nil, false
	}

	if inputBuffer[nextByteOffset] == '-' {
		nextByteOffset++
	}

	// Integer part
	if nextByteOffset >= len(inputBuffer) {
		return nil, nil, false
	}

	switch inputBuffer[nextByteOffset] {
	case '0':
		nextByteOffset++
	case '1', '2', '3', '4', '5', '6', '7', '8', '9':
		nextByteOffset++
		count, ok := readDigits(inputBuffer[nextByteOffset:], isEnding)
		if !ok {
			return nil, nil, false
		}
		nextByteOffset += count
	default:
		nextByteOffset++
		l.fileOffset += int64(nextByteOffset)
		err := l.syntaxError(l.fileOffset-1, l.fileOffset, fmt.Sprintf("A number is not allowed to start with '%c'", inputBuffer[nextByteOffset-1]))
		return nil, err, true
	}

	// Fractional part
	if nextByteOffset < len(inputBuffer) && inputBuffer[nextByteOffset] == '.' {
		nextByteOffset++
		if nextByteOffset >= len(inputBuffer) {
			if !isEnding {
				return nil, nil, false
			}
		} else {
			c := inputBuffer[nextByteOffset]
			nextByteOffset++
			if !isASCIIDigit(c) {
				l.fileOffset += int64(nextByteOffset)
				err := l.syntaxError(l.fileOffset-1, l.fileOffset, fmt.Sprintf("A number fractional part must start with a digit and not '%c'", c))
				return nil, err, true
			}
			count, ok := readDigits(inputBuffer[nextByteOffset:], isEnding)
			if !ok {
				return nil, nil, false
			}
			nextByteOffset += count
		}
	}

	// Exponent part
	if nextByteOffset < len(inputBuffer) && (inputBuffer[nextByteOffset] == 'e' || inputBuffer[nextByteOffset] == 'E') {
		nextByteOffset++
		if nextByteOffset >= len(inputBuffer) {
			return nil, nil, false
		}

		switch inputBuffer[nextByteOffset] {
		case '-', '+':
			nextByteOffset++
			if nextByteOffset >= len(inputBuffer) {
				return nil, nil, false
			}
			c := inputBuffer[nextByteOffset]
			nextByteOffset++
			if !isASCIIDigit(c) {
				l.fileOffset += int64(nextByteOffset)
				err := l.syntaxError(l.fileOffset-1, l.fileOffset, fmt.Sprintf("A number exponential part must contain at least a digit, '%c' found", c))
				return nil, err, true
			}
		case '0', '1', '2', '3', '4', '5', '6', '7', '8', '9':
			nextByteOffset++
		default:
			nextByteOffset++
			l.fileOffset += int64(nextByteOffset)
			err := l.syntaxError(l.fileOffset-1, l.fileOffset, fmt.Sprintf("A number exponential part must start with +, - or a digit, '%c' found", inputBuffer[nextByteOffset-1]))
			return nil, err, true
		}

		count, ok := readDigits(inputBuffer[nextByteOffset:], isEnding)
		if !ok {
			return nil, nil, false
		}
		nextByteOffset += count
	}

	l.fileOffset += int64(nextByteOffset)
	numberStr := string(inputBuffer[:nextByteOffset])
	return &JSONToken{Kind: TokenNumber, String: numberStr}, nil, true
}

func (l *JSONLexer) decodeUTF8(inputBuffer []byte, startPosition int64) (string, *JSONSyntaxError) {
	str := string(inputBuffer)
	if utf8.ValidString(str) {
		return str, nil
	}

	// Invalid UTF-8: replace bad sequences with U+FFFD
	runes := make([]rune, 0, len(inputBuffer))
	i := 0
	var pos int64
	for i < len(inputBuffer) {
		r, size := utf8.DecodeRune(inputBuffer[i:])
		if r == utf8.RuneError && size == 1 {
			if pos == 0 {
				pos = startPosition + int64(i)
			}
			runes = append(runes, utf8.RuneError)
		} else {
			runes = append(runes, r)
		}
		i += size
	}

	var err *JSONSyntaxError
	if pos > 0 {
		err = l.syntaxError(pos, pos+1, "Invalid UTF-8 sequence")
	}
	return string(runes), err
}

func (l *JSONLexer) syntaxError(start, end int64, message string) *JSONSyntaxError {
	startOffset := start
	if startOffset < l.fileStartOfLastLine {
		startOffset = l.fileStartOfLastLine
	}
	return &JSONSyntaxError{
		Location: struct {
			Start TextPosition
			End   TextPosition
		}{
			Start: TextPosition{
				Line:   l.fileLine,
				Column: startOffset - l.fileStartOfLastLine,
				Offset: startOffset,
			},
			End: TextPosition{
				Line:   l.fileLine,
				Column: end - l.fileStartOfLastLine,
				Offset: end,
			},
		},
		Message: message,
	}
}

// Helper functions

func isASCIIDigit(c byte) bool {
	return c >= '0' && c <= '9'
}

func isASCIIAlpha(c byte) bool {
	return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
}

func readHexaChar(input []byte) (uint32, error) {
	if len(input) < 4 {
		return 0, fmt.Errorf("Unexpected end in unicode escape sequence")
	}
	var value uint32
	for _, c := range input {
		var digit uint32
		switch {
		case c >= '0' && c <= '9':
			digit = uint32(c - '0')
		case c >= 'a' && c <= 'f':
			digit = uint32(c - 'a' + 10)
		case c >= 'A' && c <= 'F':
			digit = uint32(c - 'A' + 10)
		default:
			return 0, fmt.Errorf("Unexpected character in a unicode escape: '%c'", c)
		}
		value = value*16 + digit
	}
	return value, nil
}

func readDigits(inputBuffer []byte, isEnding bool) (int, bool) {
	count := 0
	for i := 0; i < len(inputBuffer); i++ {
		if isASCIIDigit(inputBuffer[i]) {
			count++
		} else {
			break
		}
	}
	if count == len(inputBuffer) && !isEnding {
		return 0, false
	}
	return count, true
}
