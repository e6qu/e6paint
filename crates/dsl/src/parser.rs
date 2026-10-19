//! Recursive-descent parser: tokens → [`MolDocument`], all errors collected.

use crate::ast::{BlockKind, Direction, MolBlock, MolDocument, MolLine, MolNode};
use crate::lexer::{Token, TokenKind};

/// A located parse error (1-based line/column of the offending token).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parse a mol document. Statement-level errors are collected with recovery to the next
/// line; an empty input is a valid (empty) document.
pub fn parse(input: &str) -> Result<MolDocument, Vec<ParseError>> {
    Parser {
        tokens: crate::lexer::lex(input),
        cursor: 0,
        errors: Vec::new(),
    }
    .parse_document()
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    errors: Vec<ParseError>,
}

impl Parser {
    fn parse_document(mut self) -> Result<MolDocument, Vec<ParseError>> {
        let mut blocks = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek_kind() {
                TokenKind::Eof => break,
                TokenKind::Word(word) if BlockKind::from_keyword(word).is_some() => {
                    if let Some(block) = self.parse_block() {
                        blocks.push(block);
                    }
                }
                _ => {
                    let token = self.peek().clone();
                    self.errors.push(ParseError {
                        line: token.line,
                        col: token.col,
                        message: format!(
                            "expected a block header (flow/state/mind), found {:?}",
                            token.kind
                        ),
                    });
                    self.skip_line();
                }
            }
        }
        if self.errors.is_empty() {
            Ok(MolDocument { blocks })
        } else {
            Err(self.errors)
        }
    }

    /// One block: `kind title? { line* }`.
    fn parse_block(&mut self) -> Option<MolBlock> {
        let kind = BlockKind::from_keyword(kind_word(self.peek())).unwrap_or(BlockKind::Flow);
        self.advance();

        let title = match self.peek_kind() {
            TokenKind::Str(text) => {
                let title = text.clone();
                self.advance();
                Some(title)
            }
            _ => None,
        };

        if !self.expect(TokenKind::LBrace) {
            self.skip_line();
            return None;
        }

        let mut direction = Direction::LeftRight;
        let mut lines = Vec::new();
        loop {
            match self.peek_kind() {
                TokenKind::Eof => {
                    let token = self.peek().clone();
                    self.errors.push(ParseError {
                        line: token.line,
                        col: token.col,
                        message: format!("block `{}` is missing its closing brace", kind.keyword()),
                    });
                    break;
                }
                TokenKind::RBrace => {
                    self.advance();
                    break;
                }
                TokenKind::Newline => {
                    self.advance();
                }
                _ => {
                    if let Some(line) = self.parse_line(kind, &mut direction) {
                        lines.push(line);
                    }
                }
            }
        }
        Some(MolBlock {
            kind,
            title,
            direction,
            lines,
        })
    }

    /// One statement line; errors recover to the next newline.
    fn parse_line(&mut self, kind: BlockKind, direction: &mut Direction) -> Option<MolLine> {
        let start = self.peek().clone();
        match self.peek_kind() {
            TokenKind::Word(word) if word == "direction" => {
                // Mind blocks have no direction: the printer never emits one for
                // them, so accepting it here would silently drop it on round-trip.
                if kind == BlockKind::Mind {
                    self.errors.push(ParseError {
                        line: start.line,
                        col: start.col,
                        message: "`direction:` is not valid in mind blocks".into(),
                    });
                    self.skip_line();
                    return None;
                }
                self.advance();
                if self.expect(TokenKind::Colon) {
                    let token = self.peek().clone();
                    match token {
                        Token {
                            kind: TokenKind::Word(value),
                            line,
                            col,
                        } => {
                            self.advance();
                            match value.as_str() {
                                "LR" => *direction = Direction::LeftRight,
                                "TD" => *direction = Direction::TopDown,
                                other => self.errors.push(ParseError {
                                    line,
                                    col,
                                    message: format!("unknown direction `{other}` (LR or TD)"),
                                }),
                            }
                        }
                        other => self.errors.push(ParseError {
                            line: other.line,
                            col: other.col,
                            message: format!("expected LR or TD, found {:?}", other.kind),
                        }),
                    }
                }
                self.skip_line();
                None
            }
            TokenKind::Word(id)
                if matches!(
                    kind,
                    BlockKind::Er | BlockKind::Class | BlockKind::Timeline | BlockKind::Gantt
                ) && self.peek_next_kind() == &TokenKind::Colon =>
            {
                // Row: `id: text`
                let id = id.clone();
                self.advance();
                self.advance();
                let text = self.parse_tail_text().unwrap_or_default();
                Some(MolLine::Members { id, text })
            }
            TokenKind::Str(label)
                if kind == BlockKind::Pie && self.peek_next_kind() == &TokenKind::Colon =>
            {
                // Pie row: `"label" : value`
                let label = label.clone();
                self.advance();
                self.advance();
                let text = self.parse_tail_text().unwrap_or_default();
                Some(MolLine::Members { id: label, text })
            }
            TokenKind::Word(op)
                if kind == BlockKind::Git && crate::ast::GitOpKind::from_keyword(op).is_some() =>
            {
                let Some(op_kind) = crate::ast::GitOpKind::from_keyword(op) else {
                    unreachable!("guard checked the keyword above")
                };
                self.advance();
                let arg = match self.peek().clone() {
                    Token {
                        kind: TokenKind::Word(word),
                        ..
                    } => {
                        self.advance();
                        word
                    }
                    _ => String::new(),
                };
                self.skip_line();
                Some(MolLine::GitOp { op: op_kind, arg })
            }
            TokenKind::Word(id) if kind == BlockKind::Mind => {
                let chain = self.parse_mind_chain(id.clone());
                if chain.len() >= 2 {
                    Some(MolLine::MindLink { chain })
                } else {
                    // A bare word in a mind block is a root declaration.
                    Some(MolLine::MindLink { chain })
                }
            }
            _ => {
                let first = self.parse_node_ref();
                match self.peek_kind() {
                    TokenKind::Arrow | TokenKind::ThinArrow => {
                        let dashed = self.peek_kind() == &TokenKind::Arrow;
                        self.advance();
                        let second = self.parse_node_ref();
                        let label = if self.peek_kind() == &TokenKind::Colon {
                            self.advance();
                            self.parse_tail_text()
                        } else {
                            None
                        };
                        let Some(to) = second else {
                            self.errors.push(ParseError {
                                line: start.line,
                                col: start.col,
                                message: "arrow is missing its target node".into(),
                            });
                            self.skip_line();
                            return None;
                        };
                        let Some(from) = first else {
                            self.errors.push(ParseError {
                                line: start.line,
                                col: start.col,
                                message: "arrow is missing its source node".into(),
                            });
                            self.skip_line();
                            return None;
                        };
                        if kind == BlockKind::Seq {
                            return Some(MolLine::SeqMsg {
                                from: from.id,
                                to: to.id,
                                dashed,
                                label,
                            });
                        }
                        Some(MolLine::Edge { from, to, label })
                    }
                    TokenKind::Inherits => {
                        // Inheritance syntax only exists in the er/class grammars;
                        // elsewhere it would parse into an Edge that no syntax of
                        // the block's own grammar can re-express.
                        if !matches!(kind, BlockKind::Er | BlockKind::Class) {
                            self.errors.push(ParseError {
                                line: start.line,
                                col: start.col,
                                message: "`<|--` is only valid in er and class blocks".into(),
                            });
                            self.skip_line();
                            return None;
                        }
                        self.advance();
                        let Some(parent) = first else {
                            self.errors.push(ParseError {
                                line: start.line,
                                col: start.col,
                                message: "`<|--` is missing its supertype".into(),
                            });
                            self.skip_line();
                            return None;
                        };
                        let Some(child) = self.parse_node_ref() else {
                            self.errors.push(ParseError {
                                line: start.line,
                                col: start.col,
                                message: "`<|--` is missing its subtype".into(),
                            });
                            self.skip_line();
                            return None;
                        };
                        // `A <|-- B`: B inherits from A.
                        Some(MolLine::Edge {
                            from: MolNode {
                                id: parent.id,
                                label: None,
                            },
                            to: MolNode {
                                id: child.id,
                                label: child.label,
                            },
                            label: None,
                        })
                    }
                    _ => {
                        let Some(node) = first else {
                            self.errors.push(ParseError {
                                line: start.line,
                                col: start.col,
                                message: format!("unexpected {:?}", start.kind),
                            });
                            self.skip_line();
                            return None;
                        };
                        Some(MolLine::NodeDecl {
                            id: node.id,
                            label: node.label.unwrap_or_default(),
                        })
                    }
                }
            }
        }
    }

    /// `parent > child > grandchild`
    fn parse_mind_chain(&mut self, first: String) -> Vec<String> {
        let mut chain = vec![first];
        self.advance();
        while self.peek_kind() == &TokenKind::Gt {
            self.advance();
            match self.peek().clone() {
                Token {
                    kind: TokenKind::Word(word),
                    ..
                } => {
                    self.advance();
                    chain.push(word);
                }
                other => {
                    self.errors.push(ParseError {
                        line: other.line,
                        col: other.col,
                        message: format!(
                            "`>` must be followed by a node id, found {:?}",
                            other.kind
                        ),
                    });
                    self.skip_line();
                    break;
                }
            }
        }
        self.skip_line();
        chain
    }

    /// `id` or `id[Label]`
    fn parse_node_ref(&mut self) -> Option<MolNode> {
        let Token {
            kind: TokenKind::Word(id),
            ..
        } = self.peek()
        else {
            return None;
        };
        let id = id.clone();
        self.advance();
        let label = match self.peek_kind() {
            TokenKind::LBracket => {
                self.advance();
                let text = self.parse_tail_text();
                if !self.expect(TokenKind::RBracket) {
                    return Some(MolNode { id, label: text });
                }
                text
            }
            _ => None,
        };
        Some(MolNode { id, label })
    }

    /// All remaining words/strings on the line, joined with spaces (label text).
    fn parse_tail_text(&mut self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        loop {
            match self.peek_kind() {
                TokenKind::Word(word) => {
                    parts.push(word.clone());
                    self.advance();
                }
                TokenKind::Str(text) => {
                    parts.push(text.clone());
                    self.advance();
                }
                _ => break,
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        }
    }

    fn expect(&mut self, kind: TokenKind) -> bool {
        if self.peek_kind() == &kind {
            self.advance();
            true
        } else {
            let token = self.peek().clone();
            self.errors.push(ParseError {
                line: token.line,
                col: token.col,
                message: format!("expected {kind:?}, found {:?}", token.kind),
            });
            false
        }
    }

    fn skip_line(&mut self) {
        while !matches!(self.peek_kind(), TokenKind::Newline | TokenKind::Eof) {
            self.advance();
        }
        if self.peek_kind() == &TokenKind::Newline {
            self.advance();
        }
    }

    fn skip_newlines(&mut self) {
        while self.peek_kind() == &TokenKind::Newline {
            self.advance();
        }
    }

    fn peek(&self) -> &Token {
        // Invariant: the lexer always terminates the token vector with Eof, so clamped
        // indexing is total.
        debug_assert!(!self.tokens.is_empty());
        &self.tokens[self.cursor.min(self.tokens.len() - 1)]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn peek_next_kind(&self) -> &TokenKind {
        let next = self.cursor + 1;
        if next < self.tokens.len() {
            &self.tokens[next].kind
        } else {
            &self.tokens[self.tokens.len() - 1].kind
        }
    }

    fn advance(&mut self) {
        if self.cursor + 1 < self.tokens.len() {
            self.cursor += 1;
        } else {
            self.cursor = self.tokens.len().saturating_sub(1);
        }
    }
}

fn kind_word(token: &Token) -> &str {
    match token {
        Token {
            kind: TokenKind::Word(word),
            ..
        } => word.as_str(),
        _ => "",
    }
}
