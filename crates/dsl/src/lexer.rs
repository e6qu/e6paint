//! Token kinds for the mol DSL, with line/column spans for located errors.

/// A lexed token with its 1-based source position.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

/// Lexed token kinds. Newlines are significant: one statement per line.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// Bare word: block kinds, node ids, `direction`, LR/TD values.
    Word(String),
    /// `"…"` quoted string (title text).
    Str(String),
    /// `-->`
    Arrow,
    /// `->` (sequence messages)
    ThinArrow,
    /// `<|--` (class inheritance; the `<` is consumed by the preceding word's lexer
    /// state via the word-terminator rule)
    Inherits,
    /// `>` (mind parent→child link)
    Gt,
    LBracket,
    RBracket,
    Colon,
    LBrace,
    RBrace,
    Newline,
    Eof,
}

/// Lex the whole input. `//` starts a comment running to end of line. Unterminated
/// strings are lexed as a string token ending at Eof; the parser reports them.
pub fn lex(input: &str) -> Vec<Token> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut line = 1usize;
    let mut col = 1usize;
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '\n' => {
                tokens.push(Token {
                    kind: TokenKind::Newline,
                    line,
                    col,
                });
                line += 1;
                col = 1;
                i += 1;
            }
            ' ' | '\t' | '\r' => {
                col += 1;
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    col += 1;
                    i += 1;
                }
            }
            '"' => {
                let (start_line, start_col) = (line, col);
                col += 1;
                i += 1;
                let mut text = String::new();
                while i < chars.len() && chars[i] != '"' && chars[i] != '\n' {
                    text.push(chars[i]);
                    col += 1;
                    i += 1;
                }
                if i < chars.len() && chars[i] == '"' {
                    col += 1;
                    i += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Str(text),
                    line: start_line,
                    col: start_col,
                });
            }
            '<' if chars.get(i + 1) == Some(&'|')
                && chars.get(i + 2) == Some(&'-')
                && chars.get(i + 3) == Some(&'-') =>
            {
                tokens.push(Token {
                    kind: TokenKind::Inherits,
                    line,
                    col,
                });
                col += 4;
                i += 4;
            }
            '-' => {
                let (start_line, start_col) = (line, col);
                // Longest match: `-->` then `->`.
                if chars.get(i + 1) == Some(&'-') && chars.get(i + 2) == Some(&'>') {
                    col += 3;
                    i += 3;
                    tokens.push(Token {
                        kind: TokenKind::Arrow,
                        line: start_line,
                        col: start_col,
                    });
                } else if chars.get(i + 1) == Some(&'>') {
                    col += 2;
                    i += 2;
                    tokens.push(Token {
                        kind: TokenKind::ThinArrow,
                        line: start_line,
                        col: start_col,
                    });
                } else if chars.get(i + 1) == Some(&'|') {
                    // `<|--` head is lexed as part of the word before; emit inherits edge.
                    col += 2;
                    i += 2;
                    tokens.push(Token {
                        kind: TokenKind::Inherits,
                        line: start_line,
                        col: start_col,
                    });
                } else {
                    // A stray `-` or `--` lexes as a word so the parser reports it.
                    let mut word = String::new();
                    while i < chars.len() && chars[i] == '-' {
                        word.push('-');
                        col += 1;
                        i += 1;
                    }
                    tokens.push(Token {
                        kind: TokenKind::Word(word),
                        line: start_line,
                        col: start_col,
                    });
                }
            }
            '>' => {
                tokens.push(Token {
                    kind: TokenKind::Gt,
                    line,
                    col,
                });
                col += 1;
                i += 1;
            }
            '[' => {
                tokens.push(Token {
                    kind: TokenKind::LBracket,
                    line,
                    col,
                });
                col += 1;
                i += 1;
            }
            ']' => {
                tokens.push(Token {
                    kind: TokenKind::RBracket,
                    line,
                    col,
                });
                col += 1;
                i += 1;
            }
            ':' => {
                tokens.push(Token {
                    kind: TokenKind::Colon,
                    line,
                    col,
                });
                col += 1;
                i += 1;
            }
            '{' => {
                tokens.push(Token {
                    kind: TokenKind::LBrace,
                    line,
                    col,
                });
                col += 1;
                i += 1;
            }
            '}' => {
                tokens.push(Token {
                    kind: TokenKind::RBrace,
                    line,
                    col,
                });
                col += 1;
                i += 1;
            }
            _ if is_word_char(ch) => {
                let (start_line, start_col) = (line, col);
                let mut word = String::new();
                while i < chars.len() {
                    let current = chars[i];
                    if is_word_char(current) {
                        word.push(current);
                        col += 1;
                        i += 1;
                    } else if current == '-'
                        && word.chars().next_back().is_some_and(is_word_char)
                        && chars.get(i + 1).is_some_and(|next| is_word_char(*next))
                    {
                        // Interior hyphen stays in the word (dates like 2024-01-01).
                        word.push(current);
                        col += 1;
                        i += 1;
                    } else {
                        break;
                    }
                }
                tokens.push(Token {
                    kind: TokenKind::Word(word),
                    line: start_line,
                    col: start_col,
                });
            }
            _ => {
                // Unknown characters run to the next structural boundary and surface as
                // a word the parser can report.
                let (start_line, start_col) = (line, col);
                let mut word = String::new();
                while i < chars.len()
                    && !matches!(
                        chars[i],
                        '\n' | ' '
                            | '\t'
                            | '\r'
                            | '"'
                            | '-'
                            | '>'
                            | '['
                            | ']'
                            | ':'
                            | '{'
                            | '}'
                            | '/'
                    )
                {
                    word.push(chars[i]);
                    col += 1;
                    i += 1;
                }
                if word.is_empty() {
                    col += 1;
                    i += 1;
                } else {
                    tokens.push(Token {
                        kind: TokenKind::Word(word),
                        line: start_line,
                        col: start_col,
                    });
                }
            }
        }
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        line,
        col,
    });
    tokens
}

pub(crate) fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == '!'
}
