//! Source-mapped errors with a small span type.
//!
//! The lexer, parser and interpreter all emit `LingoError` values.  We try
//! hard to give the line and column of the offending token, and to repeat
//! the source line in the message.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    pub fn dummy() -> Self {
        Self { start: 0, end: 0 }
    }
}

#[derive(Debug, Clone)]
pub struct LingoError {
    pub stage: Stage,
    pub message: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy)]
pub enum Stage {
    Lex,
    Parse,
    Resolve,
    Runtime,
}

impl LingoError {
    pub fn new(stage: Stage, message: impl Into<String>, span: Span) -> Self {
        Self { stage, message: message.into(), span }
    }

    /// Render an error against the original source string.
    pub fn render(&self, source: &str, filename: &str) -> String {
        let (line_no, col_no, line_text) = locate(source, self.span.start);
        let stage = match self.stage {
            Stage::Lex => "lex error",
            Stage::Parse => "parse error",
            Stage::Resolve => "resolve error",
            Stage::Runtime => "runtime error",
        };
        let caret_pad = " ".repeat(col_no.saturating_sub(1));
        let caret_len = (self.span.end - self.span.start).max(1);
        let caret = "^".repeat(caret_len);
        format!(
            "{stage}: {msg}\n  --> {file}:{line}:{col}\n     |\n{line:>4} | {text}\n     | {pad}{caret}",
            stage = stage,
            msg = self.message,
            file = filename,
            line = line_no,
            col = col_no,
            text = line_text,
            pad = caret_pad,
            caret = caret,
        )
    }
}

impl fmt::Display for LingoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.stage, self.message)
    }
}

impl std::error::Error for LingoError {}

/// Convert a byte offset into (1-based line, 1-based column, line text).
fn locate(source: &str, offset: usize) -> (usize, usize, &str) {
    let offset = offset.min(source.len());
    let mut line_no = 1usize;
    let mut line_start = 0usize;
    for (i, b) in source.as_bytes().iter().enumerate() {
        if i == offset {
            break;
        }
        if *b == b'\n' {
            line_no += 1;
            line_start = i + 1;
        }
    }
    let line_end = source[line_start..]
        .find('\n')
        .map(|n| line_start + n)
        .unwrap_or(source.len());
    let line_text = &source[line_start..line_end];
    let col_no = offset - line_start + 1;
    (line_no, col_no, line_text)
}
