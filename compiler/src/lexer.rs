//! Hand-written lexer for lingo v0.1.
//!
//! Indentation handling is python-like:
//! the lexer keeps a stack of indent widths and emits synthetic
//! `Indent` / `Dedent` tokens at the start of each logical line.
//! blank lines and comment-only lines never produce indent tokens.
//!
//! We only accept *spaces* for indentation in v0.1.  Mixing tabs is a hard
//! error.  This is on purpose — visible, regular indentation is part of
//! the "obvious" rule set.

use crate::error::{LingoError, Span, Stage};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    // literals
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),

    // keywords
    Fn,
    Let,
    Mut,
    Const,
    Return,
    If,
    Elif,
    Else,
    For,
    In,
    And,
    Or,
    Not,
    True,
    False,
    Then,
    Forever,
    Break,
    Continue,
    None_,
    As,
    Struct,
    Enum,
    Impl,
    Match,
    Self_,
    Print, // builtin name, treated like an identifier but reserved

    // punctuation / operators
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
    DotDot,
    Arrow,    // ->
    Assign,   // =
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar, // **
    Eq,       // ==
    Ne,       // !=
    Lt,
    Le,
    Gt,
    Ge,

    // layout
    Newline,
    Indent,
    Dedent,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

impl Token {
    pub fn new(tok: Tok, span: Span) -> Self {
        Self { tok, span }
    }
}

pub fn lex(source: &str) -> Result<Vec<Token>, LingoError> {
    let bytes = source.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut indents: Vec<usize> = vec![0];
    let mut i: usize = 0;
    let mut at_line_start = true;
    let mut paren_depth: i32 = 0; // suppress NEWLINE/INDENT inside (), [], {}

    while i < bytes.len() {
        // line-start indentation handling
        if at_line_start && paren_depth == 0 {
            let line_start = i;
            let mut width = 0usize;
            while i < bytes.len() {
                match bytes[i] {
                    b' ' => {
                        width += 1;
                        i += 1;
                    }
                    b'\t' => {
                        return Err(LingoError::new(
                            Stage::Lex,
                            "tabs are not allowed for indentation (use spaces)",
                            Span::new(i, i + 1),
                        ));
                    }
                    _ => break,
                }
            }
            // skip blank lines and comment-only lines
            if i >= bytes.len() || bytes[i] == b'\n' || bytes[i] == b'#' {
                // fall through; let the regular loop swallow newline/comment
                at_line_start = false;
                // but stay at the same `i`
                let _ = line_start;
            } else {
                let top = *indents.last().unwrap();
                if width > top {
                    indents.push(width);
                    tokens.push(Token::new(Tok::Indent, Span::new(line_start, i)));
                } else {
                    while width < *indents.last().unwrap() {
                        indents.pop();
                        tokens.push(Token::new(Tok::Dedent, Span::new(line_start, i)));
                    }
                    if width != *indents.last().unwrap() {
                        return Err(LingoError::new(
                            Stage::Lex,
                            "inconsistent indentation",
                            Span::new(line_start, i),
                        ));
                    }
                }
                at_line_start = false;
            }
            continue;
        }

        let b = bytes[i];
        let start = i;

        // skip horizontal whitespace mid-line
        if b == b' ' {
            i += 1;
            continue;
        }

        // comments: skip to end of line
        if b == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // newline
        if b == b'\n' {
            i += 1;
            if paren_depth == 0 {
                // collapse multiple newlines into one Newline token
                let already_newline = matches!(tokens.last().map(|t| &t.tok), Some(Tok::Newline) | None);
                if !already_newline {
                    tokens.push(Token::new(Tok::Newline, Span::new(start, i)));
                }
                at_line_start = true;
            }
            continue;
        }

        // string literal
        if b == b'"' {
            i += 1;
            let mut s = String::new();
            loop {
                if i >= bytes.len() {
                    return Err(LingoError::new(
                        Stage::Lex,
                        "unterminated string literal",
                        Span::new(start, i),
                    ));
                }
                let c = bytes[i];
                if c == b'"' {
                    i += 1;
                    break;
                }
                if c == b'\\' {
                    i += 1;
                    if i >= bytes.len() {
                        return Err(LingoError::new(
                            Stage::Lex,
                            "unterminated escape in string",
                            Span::new(start, i),
                        ));
                    }
                    let esc = bytes[i];
                    i += 1;
                    match esc {
                        b'n' => s.push('\n'),
                        b't' => s.push('\t'),
                        b'r' => s.push('\r'),
                        b'\\' => s.push('\\'),
                        b'"' => s.push('"'),
                        b'0' => s.push('\0'),
                        other => {
                            return Err(LingoError::new(
                                Stage::Lex,
                                format!("unknown escape '\\{}'", other as char),
                                Span::new(i - 2, i),
                            ));
                        }
                    }
                    continue;
                }
                s.push(c as char);
                i += 1;
            }
            tokens.push(Token::new(Tok::Str(s), Span::new(start, i)));
            continue;
        }

        // number literal
        if b.is_ascii_digit() {
            let mut end = i;
            let mut is_float = false;
            while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'_') {
                end += 1;
            }
            if end < bytes.len() && bytes[end] == b'.' && end + 1 < bytes.len() && bytes[end + 1].is_ascii_digit() {
                is_float = true;
                end += 1;
                while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'_') {
                    end += 1;
                }
            }
            if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
                is_float = true;
                end += 1;
                if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
                    end += 1;
                }
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
            }
            let raw: String = source[i..end].chars().filter(|c| *c != '_').collect();
            let tok = if is_float {
                Tok::Float(raw.parse::<f64>().map_err(|_| {
                    LingoError::new(Stage::Lex, "invalid float literal", Span::new(i, end))
                })?)
            } else {
                Tok::Int(raw.parse::<i64>().map_err(|_| {
                    LingoError::new(Stage::Lex, "invalid integer literal", Span::new(i, end))
                })?)
            };
            tokens.push(Token::new(tok, Span::new(i, end)));
            i = end;
            continue;
        }

        // identifier / keyword
        if b.is_ascii_alphabetic() || b == b'_' {
            let mut end = i;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            let word = &source[i..end];
            let tok = match word {
                "fn" => Tok::Fn,
                "let" => Tok::Let,
                "mut" => Tok::Mut,
                "const" => Tok::Const,
                "return" => Tok::Return,
                "if" => Tok::If,
                "elif" => Tok::Elif,
                "else" => Tok::Else,
                "for" => Tok::For,
                "in" => Tok::In,
                "and" => Tok::And,
                "or" => Tok::Or,
                "not" => Tok::Not,
                "true" => Tok::True,
                "false" => Tok::False,
                "then" => Tok::Then,
                "forever" => Tok::Forever,
                "break" => Tok::Break,
                "continue" => Tok::Continue,
                "none" => Tok::None_,
                "as" => Tok::As,
                "struct" => Tok::Struct,
                "enum" => Tok::Enum,
                "impl" => Tok::Impl,
                "match" => Tok::Match,
                "self" => Tok::Self_,
                "print" => Tok::Print,
                other => Tok::Ident(other.to_string()),
            };
            tokens.push(Token::new(tok, Span::new(i, end)));
            i = end;
            continue;
        }

        // operators / punctuation
        let two = if i + 1 < bytes.len() {
            &source[i..i + 2]
        } else {
            ""
        };
        let (tok, len) = match two {
            "->" => (Tok::Arrow, 2),
            ".." => (Tok::DotDot, 2),
            "==" => (Tok::Eq, 2),
            "!=" => (Tok::Ne, 2),
            "<=" => (Tok::Le, 2),
            ">=" => (Tok::Ge, 2),
            "**" => (Tok::StarStar, 2),
            _ => match b {
                b'(' => {
                    paren_depth += 1;
                    (Tok::LParen, 1)
                }
                b')' => {
                    paren_depth -= 1;
                    if paren_depth < 0 {
                        return Err(LingoError::new(
                            Stage::Lex,
                            "unmatched ')'",
                            Span::new(i, i + 1),
                        ));
                    }
                    (Tok::RParen, 1)
                }
                b'[' => {
                    paren_depth += 1;
                    (Tok::LBracket, 1)
                }
                b']' => {
                    paren_depth -= 1;
                    if paren_depth < 0 {
                        return Err(LingoError::new(
                            Stage::Lex,
                            "unmatched ']'",
                            Span::new(i, i + 1),
                        ));
                    }
                    (Tok::RBracket, 1)
                }
                b'{' => {
                    paren_depth += 1;
                    (Tok::LBrace, 1)
                }
                b'}' => {
                    paren_depth -= 1;
                    if paren_depth < 0 {
                        return Err(LingoError::new(
                            Stage::Lex,
                            "unmatched '}'",
                            Span::new(i, i + 1),
                        ));
                    }
                    (Tok::RBrace, 1)
                }
                b',' => (Tok::Comma, 1),
                b':' => (Tok::Colon, 1),
                b'.' => (Tok::Dot, 1),
                b'=' => (Tok::Assign, 1),
                b'+' => (Tok::Plus, 1),
                b'-' => (Tok::Minus, 1),
                b'*' => (Tok::Star, 1),
                b'/' => (Tok::Slash, 1),
                b'%' => (Tok::Percent, 1),
                b'<' => (Tok::Lt, 1),
                b'>' => (Tok::Gt, 1),
                c => {
                    return Err(LingoError::new(
                        Stage::Lex,
                        format!("unexpected character '{}'", c as char),
                        Span::new(i, i + 1),
                    ));
                }
            },
        };
        tokens.push(Token::new(tok, Span::new(i, i + len)));
        i += len;
    }

    // close any open indents at EOF
    while indents.len() > 1 {
        indents.pop();
        tokens.push(Token::new(Tok::Dedent, Span::new(bytes.len(), bytes.len())));
    }
    // ensure trailing newline so the parser sees a clean statement end
    if !matches!(tokens.last().map(|t| &t.tok), Some(Tok::Newline)) {
        tokens.push(Token::new(Tok::Newline, Span::new(bytes.len(), bytes.len())));
    }
    tokens.push(Token::new(Tok::Eof, Span::new(bytes.len(), bytes.len())));
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        lex(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn hello() {
        let toks = kinds("fn main():\n    print(\"hi\")\n");
        assert!(matches!(toks[0], Tok::Fn));
        assert!(toks.contains(&Tok::Indent));
        assert!(toks.contains(&Tok::Dedent));
        assert!(toks.iter().any(|t| matches!(t, Tok::Print)));
    }

    #[test]
    fn range() {
        let toks = kinds("for i in 0..10:\n    print(i)\n");
        assert!(toks.contains(&Tok::DotDot));
    }
}
