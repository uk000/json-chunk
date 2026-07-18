use std::borrow::Cow;
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::io;
use std::ops::Range;
use std::str;

const MAX_INDENT_STACK_SIZE: usize = 65_536;

// ─── Public Types ──────────────────────────────────────────────────────────────

/// Style in which a scalar was written in the YAML source (informational only;
/// not emitted as part of the event but used internally and exposed for callers
/// that need to distinguish, e.g., `"123"` vs `123`).
#[derive(Eq, PartialEq, Debug, Clone, Copy, Hash)]
pub enum ScalarStyle {
    Plain,
    SingleQuoted,
    DoubleQuoted,
    Literal, // |
    Folded,  // >
}

/// Events emitted by [`YAMLEventGenerator`].
///
/// The `String`, `Number`, `Boolean`, `Null`, `StartArray`/`EndArray`,
/// `StartObject`/`EndObject`, `ObjectKey`, and `Eof` events are semantically
/// identical to the corresponding [`crate::parser::JSONEvent`] variants.
///
/// `StreamStart`/`StreamEnd` and `DocumentStart`/`DocumentEnd` are YAML-only
/// extras that let callers handle multi-document streams.
#[derive(Eq, PartialEq, Debug, Clone, Hash)]
pub enum YAMLEvent<'a> {
    // ── JSON-compatible value events ──────────────────────────────────────────
    /// A string value.  Quoted scalars and block scalars always map here.
    /// Plain scalars that cannot be parsed as a number, boolean, or null also
    /// map here.
    String(Cow<'a, str>),
    /// A numeric value (integer or float, as raw text).
    Number(Cow<'a, str>),
    /// A boolean value (`true` / `false`, case-sensitive per YAML 1.2 core).
    Boolean(bool),
    /// A null value (`null`, `~`, or the empty plain scalar).
    Null,
    /// Start of an array (block sequence or flow `[`).
    StartArray,
    /// End of an array.
    EndArray,
    /// Start of an object (block mapping or flow `{`).
    StartObject,
    /// End of an object.
    EndObject,
    /// Key of a mapping entry – analogous to `JSONEvent::ObjectKey`.
    /// Always a `String`; keys are never type-classified.
    ObjectKey(Cow<'a, str>),
    /// End of the input stream.
    Eof,

    // ── YAML-only stream/document events ─────────────────────────────────────
    /// Start of the YAML byte stream.
    StreamStart,
    /// End of the YAML byte stream.
    StreamEnd,
    /// Start of a YAML document (`---` or implicit).
    DocumentStart,
    /// End of a YAML document (`...` or implicit).
    DocumentEnd,
}

/// Result returned by [`YAMLEventGenerator::next_event`].
#[derive(Debug)]
pub struct YAMLEventWrapper<'a> {
    /// Bytes consumed from `input_buffer` to produce this event.
    pub consumed_bytes: usize,
    /// The event, if one was produced.  `None` means more input is required.
    pub event: Option<Result<YAMLEvent<'a>, YAMLSyntaxError>>,
}

/// A position in the input (0-based line / column / byte offset).
#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub struct TextPosition {
    pub line: u64,
    pub column: u64,
    pub offset: u64,
}

/// A syntax error encountered while parsing YAML.
#[derive(Debug)]
pub struct YAMLSyntaxError {
    location: Range<TextPosition>,
    message: String,
}

impl YAMLSyntaxError {
    pub fn location(&self) -> Range<TextPosition> {
        self.location.clone()
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for YAMLSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = &self.location.start;
        write!(
            f,
            "YAML parse error at line {} column {}: {}",
            s.line + 1,
            s.column + 1,
            self.message
        )
    }
}

impl Error for YAMLSyntaxError {}

impl From<YAMLSyntaxError> for io::Error {
    fn from(e: YAMLSyntaxError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, e)
    }
}

/// Union of [`YAMLSyntaxError`] and [`std::io::Error`].
#[derive(Debug)]
pub enum YAMLParseError {
    Io(io::Error),
    Syntax(YAMLSyntaxError),
}

impl fmt::Display for YAMLParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Syntax(e) => e.fmt(f),
        }
    }
}

impl Error for YAMLParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(match self {
            Self::Io(e) => e,
            Self::Syntax(e) => e,
        })
    }
}

impl From<YAMLSyntaxError> for YAMLParseError {
    fn from(e: YAMLSyntaxError) -> Self {
        Self::Syntax(e)
    }
}

impl From<io::Error> for YAMLParseError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<YAMLParseError> for io::Error {
    fn from(e: YAMLParseError) -> Self {
        match e {
            YAMLParseError::Syntax(s) => s.into(),
            YAMLParseError::Io(i) => i,
        }
    }
}

// ─── Internal State ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
enum BlockContext {
    Mapping,
    Sequence,
}

#[derive(Clone, Copy, Debug)]
struct IndentEntry {
    indent: usize,
    context: BlockContext,
}

/// Chomping mode for block scalars (`|` / `>`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Chomp {
    Strip, // -  strip all trailing newlines
    Clip,  // (default) exactly one trailing newline
    Keep,  // +  keep all trailing newlines
}

#[derive(Clone, PartialEq, Debug)]
enum ParseState {
    StreamNotStarted,
    BeforeDocument,
    /// Expecting a block node at indent >= `min_indent`.
    BlockNode { min_indent: usize },
    /// Inside a block mapping; keys live at `indent`.
    BlockMappingKey { indent: usize },
    /// Emitted ObjectKey for a mapping entry; now expecting the value.
    BlockMappingValue { mapping_indent: usize },
    /// Inside a block sequence; `- ` entries live at `indent`.
    BlockSequenceEntry { indent: usize },
    /// Collecting lines for a block scalar (`|` / `>`).
    BlockScalarContent {
        style: ScalarStyle,
        content_indent: usize,
        chomp: Chomp,
    },
    // ── Flow states ───────────────────────────────────────────────────────────
    FlowMappingKey,
    FlowMappingColon,
    FlowMappingValue,
    FlowMappingCommaOrEnd,
    FlowSequenceEntry,
    FlowSequenceCommaOrEnd,
    Done,
}

// ─── Event Generator ───────────────────────────────────────────────────────────

pub struct YAMLEventGenerator {
    file_offset: u64,
    file_line: u64,
    file_start_of_last_line: u64,

    state: ParseState,
    /// Stack tracking open block collections (indent + context).
    indent_stack: Vec<IndentEntry>,
    /// Events queued to be delivered before the next scan (consumed_bytes = 0).
    pending_events: VecDeque<YAMLEvent<'static>>,
    /// Return-state stack for nested flow collections.
    /// Each entry records the ParseState to restore when the nested `}` / `]` is closed.
    flow_return_states: Vec<ParseState>,
    max_indent_stack_size: usize,
    /// Accumulation buffer for block scalar content.
    block_scalar_buf: String,
}

impl Default for YAMLEventGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl YAMLEventGenerator {
    pub const fn new() -> Self {
        Self {
            file_offset: 0,
            file_line: 0,
            file_start_of_last_line: 0,
            state: ParseState::StreamNotStarted,
            indent_stack: Vec::new(),
            pending_events: VecDeque::new(),
            flow_return_states: Vec::new(),
            max_indent_stack_size: MAX_INDENT_STACK_SIZE,
            block_scalar_buf: String::new(),
        }
    }

    /// Maximum allowed nesting depth.
    pub fn with_max_indent_stack_size(mut self, size: usize) -> Self {
        self.max_indent_stack_size = size;
        self
    }

    // ── Public entry point ────────────────────────────────────────────────────

    /// Return the next YAML event.
    ///
    /// `input_buffer` must contain all bytes not yet consumed; call this
    /// repeatedly, removing `consumed_bytes` from your buffer each time.
    /// Set `is_ending` to `true` once no more bytes will be provided.
    pub fn next_event<'a>(
        &mut self,
        input_buffer: &'a [u8],
        is_ending: bool,
    ) -> YAMLEventWrapper<'a> {
        if let Some(ev) = self.pending_events.pop_front() {
            return YAMLEventWrapper {
                consumed_bytes: 0,
                event: Some(Ok(ev)),
            };
        }

        if self.state == ParseState::StreamNotStarted {
            self.state = ParseState::BeforeDocument;
            return YAMLEventWrapper {
                consumed_bytes: 0,
                event: Some(Ok(YAMLEvent::StreamStart)),
            };
        }

        if self.state == ParseState::Done {
            return YAMLEventWrapper {
                consumed_bytes: 0,
                event: Some(Ok(YAMLEvent::Eof)),
            };
        }

        let base = self.file_offset;
        let event = self.scan(input_buffer, base, is_ending);

        YAMLEventWrapper {
            consumed_bytes: usize::try_from(self.file_offset - base).unwrap(),
            event,
        }
    }

    // ── Top-level dispatcher ───────────────────────────────────────────────────

    fn scan<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        match self.state.clone() {
            ParseState::StreamNotStarted | ParseState::Done => unreachable!(),
            ParseState::BeforeDocument => self.scan_before_document(buf, base, is_ending),
            ParseState::BlockNode { min_indent } => {
                self.scan_block_node(buf, base, is_ending, min_indent)
            }
            ParseState::BlockMappingKey { indent } => {
                self.scan_block_mapping_key(buf, base, is_ending, indent)
            }
            ParseState::BlockMappingValue { mapping_indent } => {
                self.scan_block_mapping_value(buf, base, is_ending, mapping_indent)
            }
            ParseState::BlockSequenceEntry { indent } => {
                self.scan_block_sequence_entry(buf, base, is_ending, indent)
            }
            ParseState::BlockScalarContent {
                style,
                content_indent,
                chomp,
            } => self.scan_block_scalar_content(buf, base, is_ending, style, content_indent, chomp),
            ParseState::FlowMappingKey
            | ParseState::FlowMappingColon
            | ParseState::FlowMappingValue
            | ParseState::FlowMappingCommaOrEnd
            | ParseState::FlowSequenceEntry
            | ParseState::FlowSequenceCommaOrEnd => self.scan_flow(buf, base, is_ending),
        }
    }

    // ── Before document ────────────────────────────────────────────────────────

    fn scan_before_document<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        if !self.skip_block_whitespace(buf, base, is_ending) {
            return None;
        }

        let slice = self.cur(buf, base);

        if slice.is_empty() {
            self.pending_events.push_back(YAMLEvent::StreamEnd);
            self.state = ParseState::Done;
            return Some(Ok(YAMLEvent::Eof));
        }

        // Explicit `---`
        if self.peek_doc_marker(slice, b"---", is_ending) {
            self.consume_line(buf, base, is_ending);
            self.state = ParseState::BlockNode { min_indent: 0 };
            return Some(Ok(YAMLEvent::DocumentStart));
        }

        // Implicit document start – emit DocumentStart now; content parsed next call.
        self.state = ParseState::BlockNode { min_indent: 0 };
        Some(Ok(YAMLEvent::DocumentStart))
    }

    // ── Block node (determine collection type) ─────────────────────────────────

    fn scan_block_node<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
        min_indent: usize,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        if !self.skip_block_whitespace(buf, base, is_ending) {
            return None;
        }

        let slice = self.cur(buf, base);

        if slice.is_empty() {
            return self.close_all_then_eof(is_ending);
        }

        if self.peek_doc_marker(slice, b"...", is_ending)
            || self.peek_doc_marker(slice, b"---", is_ending)
        {
            // Null node before the marker – caller will re-enter BeforeDocument.
            self.state = ParseState::BeforeDocument;
            return Some(Ok(YAMLEvent::Null));
        }

        let indent = measure_indent(slice);

        if indent < min_indent {
            // Nothing for us at this level; emit null and let parent handle the line.
            self.restore_block_state_after_value();
            return Some(Ok(YAMLEvent::Null));
        }

        let content = &slice[indent..];

        // Flow mapping `{`
        if content.first() == Some(&b'{') {
            self.advance(indent + 1);
            self.push_flow_return(ParseState::BlockMappingValue {
                mapping_indent: min_indent,
            });
            self.state = ParseState::FlowMappingKey;
            return Some(Ok(YAMLEvent::StartObject));
        }

        // Flow sequence `[`
        if content.first() == Some(&b'[') {
            self.advance(indent + 1);
            self.push_flow_return(ParseState::BlockMappingValue {
                mapping_indent: min_indent,
            });
            self.state = ParseState::FlowSequenceEntry;
            return Some(Ok(YAMLEvent::StartArray));
        }

        // Block sequence `- `
        if is_seq_entry(content) {
            if self.indent_stack.len() >= self.max_indent_stack_size {
                return Some(Err(self.make_error("Max nesting depth reached")));
            }
            self.indent_stack.push(IndentEntry {
                indent,
                context: BlockContext::Sequence,
            });
            self.state = ParseState::BlockSequenceEntry { indent };
            return Some(Ok(YAMLEvent::StartArray));
        }

        // Block mapping `key:`
        if find_block_mapping_colon(content).is_some() {
            if self.indent_stack.len() >= self.max_indent_stack_size {
                return Some(Err(self.make_error("Max nesting depth reached")));
            }
            self.indent_stack.push(IndentEntry {
                indent,
                context: BlockContext::Mapping,
            });
            self.state = ParseState::BlockMappingKey { indent };
            return Some(Ok(YAMLEvent::StartObject));
        }

        // Scalar
        self.advance(indent);
        self.scan_scalar_as_value(buf, base, is_ending, false)
    }

    // ── Block mapping key ──────────────────────────────────────────────────────

    fn scan_block_mapping_key<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
        indent: usize,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        if !self.skip_block_whitespace(buf, base, is_ending) {
            return None;
        }

        let slice = self.cur(buf, base);

        if slice.is_empty() {
            return self.close_all_then_eof(is_ending);
        }

        if self.peek_doc_marker(slice, b"...", is_ending)
            || self.peek_doc_marker(slice, b"---", is_ending)
        {
            return self.close_blocks_for_doc_marker(buf, base, is_ending);
        }

        let line_indent = measure_indent(slice);

        if line_indent < indent {
            return self.close_blocks_to_indent(line_indent, buf, base, is_ending);
        }

        let content = &slice[line_indent..];

        let colon_pos = match find_block_mapping_colon(content) {
            Some(p) => p,
            None => {
                let err = self.make_error("Expected a mapping key (key:) at this indent");
                self.advance_to_next_line(buf, base, is_ending);
                return Some(Err(err));
            }
        };

        // Consume indent + key + `:`
        self.advance(line_indent + colon_pos);
        let key_start = usize::try_from(self.file_offset - base).unwrap() - colon_pos;
        let key_end = usize::try_from(self.file_offset - base).unwrap();
        self.advance(1); // consume `:`

        let raw_key = str::from_utf8(&buf[key_start..key_end]).unwrap_or("").trim_end();
        let key = Cow::Owned(raw_key.to_owned());

        // Skip optional space after `:`
        let after = self.cur(buf, base);
        let has_inline = matches!(after.first(), Some(b) if *b != b'\n' && *b != b'\r' && *b != b'#');

        if has_inline {
            if matches!(after.first(), Some(b' ') | Some(b'\t')) {
                self.advance(1);
            }
        } else {
            self.advance_to_next_line(buf, base, is_ending);
        }

        self.state = ParseState::BlockMappingValue {
            mapping_indent: indent,
        };
        Some(Ok(YAMLEvent::ObjectKey(key)))
    }

    // ── Block mapping value ────────────────────────────────────────────────────

    fn scan_block_mapping_value<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
        mapping_indent: usize,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        // Check for inline value first (file_offset already points past the `: `)
        let inline = self.cur(buf, base);
        let has_inline = matches!(inline.first(), Some(b) if *b != b'\n' && *b != b'\r' && *b != b'#');

        if has_inline {
            return self.scan_value_or_nested(buf, base, is_ending, mapping_indent);
        }

        // No inline value – look at the next content line.
        if !self.skip_block_whitespace(buf, base, is_ending) {
            return None;
        }

        let slice = self.cur(buf, base);

        if slice.is_empty() {
            if !is_ending {
                // Buffer exhausted mid-stream; value may arrive in the next chunk.
                return None;
            }
            // EOF with no value → YAML null.
            self.state = ParseState::BlockMappingKey {
                indent: mapping_indent,
            };
            self.pending_events.push_back(YAMLEvent::EndObject);
            return Some(Ok(YAMLEvent::Null));
        }

        if self.peek_doc_marker(slice, b"...", is_ending)
            || self.peek_doc_marker(slice, b"---", is_ending)
        {
            self.state = ParseState::BlockMappingKey {
                indent: mapping_indent,
            };
            return Some(Ok(YAMLEvent::Null));
        }

        let line_indent = measure_indent(slice);

        if line_indent <= mapping_indent {
            // Same or outer indent → null value.
            self.state = ParseState::BlockMappingKey {
                indent: mapping_indent,
            };
            return Some(Ok(YAMLEvent::Null));
        }

        // Deeper indent → nested node.
        self.scan_value_or_nested(buf, base, is_ending, mapping_indent)
    }

    /// Scan a value that may be a nested collection or a scalar.
    fn scan_value_or_nested<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
        _parent_indent: usize,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        let slice = self.cur(buf, base);

        if slice.is_empty() {
            if !is_ending {
                return None;
            }
            self.restore_block_state_after_value();
            return Some(Ok(YAMLEvent::Null));
        }

        let indent = measure_indent(slice);
        let content = &slice[indent..];

        // Block scalar `|` / `>`
        if let Some((bs_style, bs_chomp)) = is_block_scalar_indicator(content) {
            self.advance(indent + 1);
            self.consume_block_scalar_header(buf, base, is_ending);
            self.block_scalar_buf.clear();
            self.state = ParseState::BlockScalarContent {
                style: bs_style,
                content_indent: usize::MAX, // auto-detect from first content line
                chomp: bs_chomp,
            };
            return self.scan_block_scalar_content(
                buf,
                base,
                is_ending,
                bs_style,
                usize::MAX,
                bs_chomp,
            );
        }

        // Flow mapping `{`
        if content.first() == Some(&b'{') {
            self.advance(indent + 1);
            let ret = self.block_return_state_after_value();
            self.push_flow_return(ret);
            self.state = ParseState::FlowMappingKey;
            return Some(Ok(YAMLEvent::StartObject));
        }

        // Flow sequence `[`
        if content.first() == Some(&b'[') {
            self.advance(indent + 1);
            let ret = self.block_return_state_after_value();
            self.push_flow_return(ret);
            self.state = ParseState::FlowSequenceEntry;
            return Some(Ok(YAMLEvent::StartArray));
        }

        // Block sequence `- `
        if is_seq_entry(content) {
            if self.indent_stack.len() >= self.max_indent_stack_size {
                return Some(Err(self.make_error("Max nesting depth reached")));
            }
            self.indent_stack.push(IndentEntry {
                indent,
                context: BlockContext::Sequence,
            });
            self.state = ParseState::BlockSequenceEntry { indent };
            return Some(Ok(YAMLEvent::StartArray));
        }

        // Block mapping `key:`
        if find_block_mapping_colon(content).is_some() {
            if self.indent_stack.len() >= self.max_indent_stack_size {
                return Some(Err(self.make_error("Max nesting depth reached")));
            }
            self.indent_stack.push(IndentEntry {
                indent,
                context: BlockContext::Mapping,
            });
            self.state = ParseState::BlockMappingKey { indent };
            return Some(Ok(YAMLEvent::StartObject));
        }

        // Scalar value
        self.advance(indent);
        self.scan_scalar_as_value(buf, base, is_ending, false)
    }

    // ── Block sequence entry ───────────────────────────────────────────────────

    fn scan_block_sequence_entry<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
        indent: usize,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        if !self.skip_block_whitespace(buf, base, is_ending) {
            return None;
        }

        let slice = self.cur(buf, base);

        if slice.is_empty() {
            return self.close_all_then_eof(is_ending);
        }

        if self.peek_doc_marker(slice, b"...", is_ending)
            || self.peek_doc_marker(slice, b"---", is_ending)
        {
            return self.close_blocks_for_doc_marker(buf, base, is_ending);
        }

        let line_indent = measure_indent(slice);

        if line_indent < indent {
            return self.close_blocks_to_indent(line_indent, buf, base, is_ending);
        }

        let content = &slice[line_indent..];

        if !is_seq_entry(content) {
            let err = self.make_error("Expected a sequence entry `- ` at this indent");
            self.advance_to_next_line(buf, base, is_ending);
            return Some(Err(err));
        }

        // Consume `<indent>- `
        self.advance(line_indent + 1); // spaces + `-`
        if matches!(self.cur(buf, base).first(), Some(b' ') | Some(b'\t')) {
            self.advance(1);
        }

        let val_slice = self.cur(buf, base);
        let no_inline = val_slice.is_empty()
            || matches!(val_slice[0], b'\n' | b'\r' | b'#');

        if no_inline {
            self.advance_to_next_line(buf, base, is_ending);
            // Value is on subsequent lines – re-enter as a nested node.
            return self.scan(buf, base, is_ending);
        }

        self.scan_value_or_nested(buf, base, is_ending, indent)
    }

    // ── Block scalar content ────────────────────────────────────────────────────

    fn scan_block_scalar_content<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
        style: ScalarStyle,
        mut content_indent: usize,
        chomp: Chomp,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        loop {
            let slice = self.cur(buf, base);
            let line_end = match find_newline(slice) {
                Some(p) => p,
                None => {
                    if is_ending {
                        slice.len()
                    } else {
                        return None;
                    }
                }
            };

            let line = &slice[..line_end];
            let is_blank = line.iter().all(|b| *b == b' ' || *b == b'\t');

            if is_blank {
                self.block_scalar_buf.push('\n');
                self.consume_bytes_and_newline(buf, base, line_end);
                continue;
            }

            let line_indent = measure_indent(line);

            // Auto-detect content indent from first non-blank line.
            if content_indent == usize::MAX {
                content_indent = line_indent;
                self.state = ParseState::BlockScalarContent {
                    style,
                    content_indent,
                    chomp,
                };
            }

            if line_indent < content_indent {
                break; // dedent → end of block scalar
            }

            let text = str::from_utf8(&line[content_indent..]).unwrap_or("");
            match style {
                ScalarStyle::Literal => {
                    self.block_scalar_buf.push_str(text);
                    self.block_scalar_buf.push('\n');
                }
                ScalarStyle::Folded => {
                    if !self.block_scalar_buf.is_empty()
                        && !self.block_scalar_buf.ends_with('\n')
                    {
                        self.block_scalar_buf.push(' ');
                    }
                    self.block_scalar_buf.push_str(text);
                }
                _ => unreachable!(),
            }
            self.consume_bytes_and_newline(buf, base, line_end);
        }

        let result = apply_chomp(&self.block_scalar_buf, chomp).to_owned();
        self.block_scalar_buf.clear();
        self.restore_block_state_after_value();

        Some(Ok(YAMLEvent::String(Cow::Owned(result))))
    }

    // ── Flow scanning ──────────────────────────────────────────────────────────

    fn scan_flow<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        self.skip_flow_whitespace(buf, base);

        let slice = self.cur(buf, base);
        if slice.is_empty() {
            if is_ending {
                return Some(Err(
                    self.make_error("Unexpected end of input inside flow collection")
                ));
            }
            return None;
        }

        match self.state {
            ParseState::FlowMappingKey => self.scan_flow_mapping_key(buf, base, is_ending),
            ParseState::FlowMappingColon => self.scan_flow_mapping_colon(buf, base, is_ending),
            ParseState::FlowMappingValue => self.scan_flow_mapping_value(buf, base, is_ending),
            ParseState::FlowMappingCommaOrEnd => {
                self.scan_flow_mapping_comma_or_end(buf, base, is_ending)
            }
            ParseState::FlowSequenceEntry => self.scan_flow_sequence_entry(buf, base, is_ending),
            ParseState::FlowSequenceCommaOrEnd => {
                self.scan_flow_sequence_comma_or_end(buf, base, is_ending)
            }
            _ => unreachable!(),
        }
    }

    fn scan_flow_mapping_key<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        if self.cur(buf, base).first() == Some(&b'}') {
            self.advance(1);
            let ret = self.pop_flow_return();
            self.state = ret;
            return Some(Ok(YAMLEvent::EndObject));
        }

        let result = self.read_flow_scalar(buf, base, is_ending)?;
        match result {
            Ok((key, _style)) => {
                self.state = ParseState::FlowMappingColon;
                Some(Ok(YAMLEvent::ObjectKey(key)))
            }
            Err(e) => Some(Err(e)),
        }
    }

    fn scan_flow_mapping_colon<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        if self.cur(buf, base).first() != Some(&b':') {
            return Some(Err(self.make_error("Expected ':' after flow mapping key")));
        }
        self.advance(1);
        self.skip_flow_whitespace(buf, base);
        self.state = ParseState::FlowMappingValue;
        self.scan_flow_mapping_value(buf, base, is_ending)
    }

    fn scan_flow_mapping_value<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        let slice = self.cur(buf, base);

        if slice.first() == Some(&b'{') {
            self.advance(1);
            // When this nested mapping ends, return to FlowMappingCommaOrEnd.
            self.push_flow_return(ParseState::FlowMappingCommaOrEnd);
            self.state = ParseState::FlowMappingKey;
            return Some(Ok(YAMLEvent::StartObject));
        }

        if slice.first() == Some(&b'[') {
            self.advance(1);
            self.push_flow_return(ParseState::FlowMappingCommaOrEnd);
            self.state = ParseState::FlowSequenceEntry;
            return Some(Ok(YAMLEvent::StartArray));
        }

        let result = self.read_flow_scalar(buf, base, is_ending)?;
        match result {
            Ok((val, style)) => {
                self.state = ParseState::FlowMappingCommaOrEnd;
                Some(Ok(emit_scalar(val, style)))
            }
            Err(e) => Some(Err(e)),
        }
    }

    fn scan_flow_mapping_comma_or_end<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        match self.cur(buf, base).first() {
            Some(&b',') => {
                self.advance(1);
                self.skip_flow_whitespace(buf, base);
                self.state = ParseState::FlowMappingKey;
                self.scan_flow_mapping_key(buf, base, is_ending)
            }
            Some(&b'}') => {
                self.advance(1);
                let ret = self.pop_flow_return();
                self.state = ret;
                Some(Ok(YAMLEvent::EndObject))
            }
            _ => Some(Err(self.make_error("Expected ',' or '}' in flow mapping"))),
        }
    }

    fn scan_flow_sequence_entry<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        if self.cur(buf, base).first() == Some(&b']') {
            self.advance(1);
            let ret = self.pop_flow_return();
            self.state = ret;
            return Some(Ok(YAMLEvent::EndArray));
        }

        let slice = self.cur(buf, base);

        if slice.first() == Some(&b'{') {
            self.advance(1);
            self.push_flow_return(ParseState::FlowSequenceCommaOrEnd);
            self.state = ParseState::FlowMappingKey;
            return Some(Ok(YAMLEvent::StartObject));
        }

        if slice.first() == Some(&b'[') {
            self.advance(1);
            self.push_flow_return(ParseState::FlowSequenceCommaOrEnd);
            self.state = ParseState::FlowSequenceEntry;
            return Some(Ok(YAMLEvent::StartArray));
        }

        let result = self.read_flow_scalar(buf, base, is_ending)?;
        match result {
            Ok((val, style)) => {
                self.state = ParseState::FlowSequenceCommaOrEnd;
                Some(Ok(emit_scalar(val, style)))
            }
            Err(e) => Some(Err(e)),
        }
    }

    fn scan_flow_sequence_comma_or_end<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        match self.cur(buf, base).first() {
            Some(&b',') => {
                self.advance(1);
                self.skip_flow_whitespace(buf, base);
                self.state = ParseState::FlowSequenceEntry;
                self.scan_flow_sequence_entry(buf, base, is_ending)
            }
            Some(&b']') => {
                self.advance(1);
                let ret = self.pop_flow_return();
                self.state = ret;
                Some(Ok(YAMLEvent::EndArray))
            }
            _ => Some(Err(self.make_error("Expected ',' or ']' in flow sequence"))),
        }
    }

    // ── Flow scalar reader ─────────────────────────────────────────────────────

    /// Read a scalar in flow context.  Returns `(value, style)`.
    fn read_flow_scalar<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        _is_ending: bool,
    ) -> Option<Result<(Cow<'a, str>, ScalarStyle), YAMLSyntaxError>> {
        let slice = self.cur(buf, base);

        match slice.first()? {
            b'"' => self
                .read_double_quoted(buf, base)
                .map(|r| r.map(|s| (s, ScalarStyle::DoubleQuoted))),
            b'\'' => self
                .read_single_quoted(buf, base)
                .map(|r| r.map(|s| (s, ScalarStyle::SingleQuoted))),
            _ => {
                let mut i = 0;
                while i < slice.len() {
                    match slice[i] {
                        b',' | b'}' | b']' => break,
                        b'#' if i > 0
                            && matches!(slice[i - 1], b' ' | b'\t') =>
                        {
                            break
                        }
                        b':' if matches!(
                            slice.get(i + 1),
                            Some(b' ') | Some(b'\t') | Some(b'\n') | None
                        ) =>
                        {
                            break
                        }
                        b'\n' | b'\r' => break,
                        _ => i += 1,
                    }
                }
                let raw = str::from_utf8(&slice[..i]).unwrap_or("").trim_end();
                let owned = raw.to_owned();
                self.advance(i);
                Some(Ok((Cow::Owned(owned), ScalarStyle::Plain)))
            }
        }
    }

    fn read_double_quoted<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
    ) -> Option<Result<Cow<'a, str>, YAMLSyntaxError>> {
        let slice = self.cur(buf, base);
        debug_assert_eq!(slice[0], b'"');
        let mut result = String::new();
        let mut i = 1;
        loop {
            let c = *slice.get(i)?;
            match c {
                b'"' => {
                    i += 1;
                    self.advance(i);
                    return Some(Ok(Cow::Owned(result)));
                }
                b'\\' => {
                    i += 1;
                    let esc = *slice.get(i)?;
                    i += 1;
                    match esc {
                        b'"' => result.push('"'),
                        b'\\' => result.push('\\'),
                        b'/' => result.push('/'),
                        b'n' => result.push('\n'),
                        b'r' => result.push('\r'),
                        b't' => result.push('\t'),
                        b'b' => result.push('\u{08}'),
                        b'f' => result.push('\u{0C}'),
                        b'u' => {
                            let hex = slice.get(i..i + 4)?;
                            i += 4;
                            let cp = u32::from_str_radix(
                                str::from_utf8(hex).unwrap_or(""),
                                16,
                            )
                            .unwrap_or(0xFFFD);
                            result.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        b'U' => {
                            let hex = slice.get(i..i + 8)?;
                            i += 8;
                            let cp = u32::from_str_radix(
                                str::from_utf8(hex).unwrap_or(""),
                                16,
                            )
                            .unwrap_or(0xFFFD);
                            result.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        b'\n' => {
                            while matches!(slice.get(i), Some(b' ') | Some(b'\t')) {
                                i += 1;
                            }
                        }
                        _ => result.push(char::from(esc)),
                    }
                }
                b'\n' => {
                    result.push(' ');
                    i += 1;
                }
                _ => {
                    result.push(char::from(c));
                    i += 1;
                }
            }
        }
    }

    fn read_single_quoted<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
    ) -> Option<Result<Cow<'a, str>, YAMLSyntaxError>> {
        let slice = self.cur(buf, base);
        debug_assert_eq!(slice[0], b'\'');
        let mut result = String::new();
        let mut i = 1;
        loop {
            let c = *slice.get(i)?;
            match c {
                b'\'' => {
                    i += 1;
                    if slice.get(i) == Some(&b'\'') {
                        result.push('\'');
                        i += 1;
                    } else {
                        self.advance(i);
                        return Some(Ok(Cow::Owned(result)));
                    }
                }
                b'\n' => {
                    result.push(' ');
                    i += 1;
                }
                _ => {
                    result.push(char::from(c));
                    i += 1;
                }
            }
        }
    }

    // ── Scalar as a value (block context) ─────────────────────────────────────

    fn scan_scalar_as_value<'a>(
        &mut self,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
        _is_key: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        let slice = self.cur(buf, base);

        // Block scalar
        if let Some((style, chomp)) = is_block_scalar_indicator(slice) {
            self.advance(1);
            self.consume_block_scalar_header(buf, base, is_ending);
            self.block_scalar_buf.clear();
            self.state = ParseState::BlockScalarContent {
                style,
                content_indent: usize::MAX,
                chomp,
            };
            return self.scan_block_scalar_content(buf, base, is_ending, style, usize::MAX, chomp);
        }

        // Double-quoted
        if slice.first() == Some(&b'"') {
            let res = self.read_double_quoted(buf, base)?;
            self.restore_block_state_after_value();
            return Some(res.map(|v| YAMLEvent::String(v)));
        }

        // Single-quoted
        if slice.first() == Some(&b'\'') {
            let res = self.read_single_quoted(buf, base)?;
            self.restore_block_state_after_value();
            return Some(res.map(|v| YAMLEvent::String(v)));
        }

        // Plain scalar – needs a complete line.
        let line_end = match find_newline(slice) {
            Some(p) => p,
            None => {
                if is_ending {
                    slice.len()
                } else {
                    return None;
                }
            }
        };

        let raw = trim_plain_scalar(&slice[..line_end]);
        let owned = raw.to_owned();
        self.consume_bytes_and_newline(buf, base, line_end);
        self.restore_block_state_after_value();

        Some(Ok(emit_scalar(Cow::Owned(owned), ScalarStyle::Plain)))
    }

    // ── State helpers ──────────────────────────────────────────────────────────

    /// After a block value is emitted, return to the enclosing block's state.
    fn restore_block_state_after_value(&mut self) {
        match self.indent_stack.last() {
            Some(e) => {
                self.state = match e.context {
                    BlockContext::Mapping => ParseState::BlockMappingKey { indent: e.indent },
                    BlockContext::Sequence => ParseState::BlockSequenceEntry { indent: e.indent },
                };
            }
            None => {
                self.pending_events.push_back(YAMLEvent::DocumentEnd);
                self.state = ParseState::BeforeDocument;
            }
        }
    }

    /// Compute the block return state for after a value without modifying state.
    fn block_return_state_after_value(&self) -> ParseState {
        match self.indent_stack.last() {
            Some(e) => match e.context {
                BlockContext::Mapping => ParseState::BlockMappingKey { indent: e.indent },
                BlockContext::Sequence => ParseState::BlockSequenceEntry { indent: e.indent },
            },
            None => ParseState::BeforeDocument,
        }
    }

    /// Push a return state onto the flow return stack.
    fn push_flow_return(&mut self, ret: ParseState) {
        self.flow_return_states.push(ret);
    }

    /// Pop a return state from the flow return stack (fallback: BeforeDocument).
    fn pop_flow_return(&mut self) -> ParseState {
        self.flow_return_states
            .pop()
            .unwrap_or(ParseState::BeforeDocument)
    }

    // ── Block closing helpers ──────────────────────────────────────────────────

    fn close_all_then_eof(
        &mut self,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'static>, YAMLSyntaxError>> {
        if !is_ending {
            return None;
        }
        while let Some(entry) = self.indent_stack.pop() {
            self.pending_events.push_back(match entry.context {
                BlockContext::Mapping => YAMLEvent::EndObject,
                BlockContext::Sequence => YAMLEvent::EndArray,
            });
        }
        self.pending_events.push_back(YAMLEvent::DocumentEnd);
        self.pending_events.push_back(YAMLEvent::StreamEnd);
        self.state = ParseState::Done;
        Some(Ok(YAMLEvent::Eof))
    }

    fn close_blocks_to_indent<'a>(
        &mut self,
        target_indent: usize,
        buf: &'a [u8],
        base: u64,
        is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        // Pop until we reach a block at or below target_indent.
        let first = loop {
            match self.indent_stack.last() {
                Some(e) if e.indent > target_indent => {
                    let entry = self.indent_stack.pop().unwrap();
                    let ev: YAMLEvent<'static> = match entry.context {
                        BlockContext::Mapping => YAMLEvent::EndObject,
                        BlockContext::Sequence => YAMLEvent::EndArray,
                    };
                    // We need to return the first End now; queue the rest.
                    // Check if we need to keep popping.
                    if self
                        .indent_stack
                        .last()
                        .map_or(true, |e| e.indent <= target_indent)
                    {
                        break ev;
                    }
                    self.pending_events.push_back(ev);
                }
                _ => {
                    // Nothing to close; just re-scan at this state.
                    return self.scan(buf, base, is_ending);
                }
            }
        };

        // Update state to the new top of stack.
        match self.indent_stack.last() {
            Some(e) => {
                self.state = match e.context {
                    BlockContext::Mapping => ParseState::BlockMappingKey { indent: e.indent },
                    BlockContext::Sequence => ParseState::BlockSequenceEntry { indent: e.indent },
                };
            }
            None => {
                // Returned to document level – check if there's more content.
                let slice = self.cur(buf, base);
                if slice.is_empty() {
                    self.pending_events.push_back(YAMLEvent::DocumentEnd);
                    self.pending_events.push_back(YAMLEvent::StreamEnd);
                    self.state = ParseState::Done;
                } else {
                    self.pending_events.push_back(YAMLEvent::DocumentEnd);
                    self.state = ParseState::BeforeDocument;
                }
            }
        }

        Some(Ok(first))
    }

    fn close_blocks_for_doc_marker<'a>(
        &mut self,
        buf: &'a [u8],
        _base: u64,
        _is_ending: bool,
    ) -> Option<Result<YAMLEvent<'a>, YAMLSyntaxError>> {
        if let Some(entry) = self.indent_stack.pop() {
            let first: YAMLEvent<'static> = match entry.context {
                BlockContext::Mapping => YAMLEvent::EndObject,
                BlockContext::Sequence => YAMLEvent::EndArray,
            };
            while let Some(e) = self.indent_stack.pop() {
                self.pending_events.push_back(match e.context {
                    BlockContext::Mapping => YAMLEvent::EndObject,
                    BlockContext::Sequence => YAMLEvent::EndArray,
                });
            }
            self.pending_events.push_back(YAMLEvent::DocumentEnd);
            self.state = ParseState::BeforeDocument;
            return Some(Ok(first));
        }
        self.pending_events.push_back(YAMLEvent::DocumentEnd);
        self.state = ParseState::BeforeDocument;
        // Re-enter to process the `---` / `...` marker.
        self.scan(buf, _base, _is_ending)
    }

    // ── Low-level movement ─────────────────────────────────────────────────────

    #[inline]
    fn cur<'a>(&self, buf: &'a [u8], base: u64) -> &'a [u8] {
        &buf[usize::try_from(self.file_offset - base).unwrap()..]
    }

    #[inline]
    fn advance(&mut self, n: usize) {
        self.file_offset += u64::try_from(n).unwrap();
    }

    fn consume_bytes_and_newline(&mut self, buf: &[u8], base: u64, n: usize) {
        self.advance(n);
        let slice = self.cur(buf, base);
        if let Some(&b'\r') = slice.first() {
            self.advance(1);
            if self.cur(buf, base).first() == Some(&b'\n') {
                self.advance(1);
            }
        } else if let Some(&b'\n') = slice.first() {
            self.advance(1);
        }
        self.file_line += 1;
        self.file_start_of_last_line = self.file_offset;
    }

    fn advance_to_next_line(&mut self, buf: &[u8], base: u64, is_ending: bool) {
        let slice = self.cur(buf, base);
        let n = find_newline(slice).unwrap_or(if is_ending { slice.len() } else { 0 });
        if n > 0 || is_ending {
            self.consume_bytes_and_newline(buf, base, n);
        }
    }

    fn consume_line(&mut self, buf: &[u8], base: u64, is_ending: bool) {
        self.advance_to_next_line(buf, base, is_ending);
    }

    fn consume_block_scalar_header(&mut self, buf: &[u8], base: u64, is_ending: bool) {
        self.advance_to_next_line(buf, base, is_ending);
    }

    // ── Whitespace skipping ────────────────────────────────────────────────────

    /// Skip blank lines and comment lines in block context.
    /// Returns `false` if more input is needed (mid-line at end of buffer).
    fn skip_block_whitespace(&mut self, buf: &[u8], base: u64, is_ending: bool) -> bool {
        loop {
            let slice = self.cur(buf, base);
            if slice.is_empty() {
                return true;
            }
            let line_end = match find_newline(slice) {
                Some(p) => p,
                None => {
                    if is_ending {
                        slice.len()
                    } else {
                        let all_blank = slice.iter().all(|b| *b == b' ' || *b == b'\t');
                        return !all_blank; // true = stop here (has content); false = need more
                    }
                }
            };
            let line = &slice[..line_end];
            match line.iter().position(|b| *b != b' ' && *b != b'\t') {
                None => {
                    // Blank line
                    self.consume_bytes_and_newline(buf, base, line_end);
                }
                Some(i) if line[i] == b'#' => {
                    // Comment
                    self.consume_bytes_and_newline(buf, base, line_end);
                }
                _ => return true,
            }
        }
    }

    fn skip_flow_whitespace(&mut self, buf: &[u8], base: u64) {
        let slice = self.cur(buf, base);
        for &b in slice {
            match b {
                b' ' | b'\t' => {
                    self.advance(1);
                }
                b'\n' | b'\r' => {
                    self.advance(1);
                    self.file_line += 1;
                    self.file_start_of_last_line = self.file_offset;
                }
                _ => break,
            }
        }
    }

    // ── Misc ──────────────────────────────────────────────────────────────────

    fn peek_doc_marker(&self, slice: &[u8], marker: &[u8; 3], is_ending: bool) -> bool {
        if !slice.starts_with(marker.as_slice()) {
            return false;
        }
        match slice.get(3) {
            None => is_ending,
            Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'#') => true,
            _ => false,
        }
    }

    fn make_error(&self, msg: &str) -> YAMLSyntaxError {
        let col = self.file_offset - self.file_start_of_last_line;
        let pos = TextPosition {
            line: self.file_line,
            column: col,
            offset: self.file_offset,
        };
        YAMLSyntaxError {
            location: pos..pos,
            message: msg.to_owned(),
        }
    }
}

// ─── Free helpers ──────────────────────────────────────────────────────────────

/// Count leading spaces/tabs.
fn measure_indent(line: &[u8]) -> usize {
    line.iter()
        .take_while(|b| **b == b' ' || **b == b'\t')
        .count()
}

fn find_newline(slice: &[u8]) -> Option<usize> {
    slice.iter().position(|b| *b == b'\n' || *b == b'\r')
}

fn is_seq_entry(content: &[u8]) -> bool {
    content.first() == Some(&b'-')
        && matches!(
            content.get(1),
            Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | None
        )
}

/// Return the byte position of the `:` that acts as a YAML block mapping
/// indicator (followed by space/tab/newline/comment/EOF), respecting quoted
/// sections so that `url: http://x` doesn't split on the second colon.
fn find_block_mapping_colon(content: &[u8]) -> Option<usize> {
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < content.len() {
        if in_single {
            if content[i] == b'\'' {
                i += if content.get(i + 1) == Some(&b'\'') {
                    2
                } else {
                    in_single = false;
                    1
                };
                continue;
            }
        } else if in_double {
            if content[i] == b'\\' {
                i += 2;
                continue;
            }
            if content[i] == b'"' {
                in_double = false;
            }
        } else {
            match content[i] {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'#' => return None,
                b':' => {
                    if matches!(
                        content.get(i + 1),
                        Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'#') | None
                    ) {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn is_block_scalar_indicator(content: &[u8]) -> Option<(ScalarStyle, Chomp)> {
    let style = match content.first() {
        Some(b'|') => ScalarStyle::Literal,
        Some(b'>') => ScalarStyle::Folded,
        _ => return None,
    };
    let chomp = match content.get(1) {
        Some(b'-') => Chomp::Strip,
        Some(b'+') => Chomp::Keep,
        _ => Chomp::Clip,
    };
    Some((style, chomp))
}

/// Trim trailing whitespace and inline comments from a plain scalar line.
fn trim_plain_scalar(line: &[u8]) -> &str {
    let mut end = line.len();
    for i in 0..line.len() {
        if line[i] == b'#' && i > 0 && matches!(line[i - 1], b' ' | b'\t') {
            end = i;
            break;
        }
    }
    str::from_utf8(&line[..end]).unwrap_or("").trim_end()
}

fn apply_chomp<'a>(s: &'a str, chomp: Chomp) -> &'a str {
    match chomp {
        Chomp::Strip => s.trim_end_matches('\n'),
        Chomp::Clip => s.trim_end_matches('\n'), // caller adds one \n if needed
        Chomp::Keep => s,
    }
}

/// Classify a plain scalar as the appropriate typed event.
///
/// Uses YAML 1.2 Core Schema rules (JSON-compatible): only `true`/`false` are
/// booleans; `yes`/`no`/`on`/`off` are plain strings.
fn emit_scalar(val: Cow<'_, str>, style: ScalarStyle) -> YAMLEvent<'_> {
    match style {
        // Quoted and block scalars are always strings.
        ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted | ScalarStyle::Literal | ScalarStyle::Folded => {
            YAMLEvent::String(val)
        }
        ScalarStyle::Plain => match val.as_ref() {
            "true" | "True" | "TRUE" => YAMLEvent::Boolean(true),
            "false" | "False" | "FALSE" => YAMLEvent::Boolean(false),
            "null" | "Null" | "NULL" | "~" | "" => YAMLEvent::Null,
            s if is_yaml_number(s) => YAMLEvent::Number(val),
            _ => YAMLEvent::String(val),
        },
    }
}

/// Return `true` if `s` looks like a YAML 1.2 Core number.
pub fn is_yaml_number(s: &str) -> bool {
    // Integer: optional sign + digits (+ 0o/0x/0b prefixes)
    // Float: optional sign + digits + optional fractional/exp, or .inf / .nan
    let s = s.trim();
    if s.is_empty() {
        return false;
    }

    // Special float literals
    if matches!(
        s,
        ".inf" | ".Inf" | ".INF" | "-.inf" | "-.Inf" | "-.INF" | "+.inf" | "+.Inf" | "+.INF"
            | ".nan" | ".NaN" | ".NAN"
    ) {
        return true;
    }

    let bytes = s.as_bytes();
    let start = if matches!(bytes[0], b'+' | b'-') { 1 } else { 0 };
    let rest = &bytes[start..];

    if rest.is_empty() {
        return false;
    }

    // Hex 0x…
    if rest.starts_with(b"0x") || rest.starts_with(b"0X") {
        return rest[2..].iter().all(|b| b.is_ascii_hexdigit());
    }
    // Octal 0o…
    if rest.starts_with(b"0o") || rest.starts_with(b"0O") {
        return rest[2..].iter().all(|b| matches!(b, b'0'..=b'7'));
    }
    // Binary 0b…
    if rest.starts_with(b"0b") || rest.starts_with(b"0B") {
        return rest[2..].iter().all(|b| *b == b'0' || *b == b'1');
    }

    // Decimal integer or float
    let mut i = 0;
    while i < rest.len() && rest[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return false;
    }
    if i == rest.len() {
        return true; // plain integer
    }

    // Optional fractional part
    if rest[i] == b'.' {
        i += 1;
        while i < rest.len() && rest[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == rest.len() {
        return true;
    }

    // Optional exponent
    if matches!(rest[i], b'e' | b'E') {
        i += 1;
        if i < rest.len() && matches!(rest[i], b'+' | b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < rest.len() && rest[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return false; // exponent with no digits
        }
    }

    i == rest.len()
}
