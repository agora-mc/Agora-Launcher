use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum VdfValue {
    String(String),
    Object(Vec<(String, VdfValue)>),
}

impl VdfValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            VdfValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, VdfValue)]> {
        match self {
            VdfValue::Object(obj) => Some(obj.as_slice()),
            _ => None,
        }
    }
}

pub fn get_str<'a>(entries: &'a [(String, VdfValue)], key: &str) -> Option<&'a str> {
    for (k, v) in entries {
        if k.eq_ignore_ascii_case(key) {
            return v.as_str();
        }
    }
    None
}

pub fn get_object<'a>(
    entries: &'a [(String, VdfValue)],
    key: &str,
) -> Option<&'a [(String, VdfValue)]> {
    for (k, v) in entries {
        if k.eq_ignore_ascii_case(key) {
            return v.as_object();
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VdfParseError {
    pub message: String,
    pub line: usize,
}

impl fmt::Display for VdfParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for VdfParseError {}

struct VdfLexer<'a> {
    _marker: std::marker::PhantomData<&'a ()>,
    chars: Vec<(usize, char)>, // (byte_offset, char)
    pos: usize,
    line: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    String(String),
    OpenBrace,
    CloseBrace,
}

impl<'a> VdfLexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            _marker: std::marker::PhantomData,
            chars: input.char_indices().collect(),
            pos: 0,
            line: 1,
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.chars.get(self.pos).map(|&(_, c)| c)
    }

    fn next_char(&mut self) -> Option<char> {
        if let Some(&(_, c)) = self.chars.get(self.pos) {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
            }
            Some(c)
        } else {
            None
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() {
                self.next_char();
            } else if c == '/' {
                if self.pos + 1 < self.chars.len() && self.chars[self.pos + 1].1 == '/' {
                    // Line comment
                    self.next_char();
                    self.next_char();
                    while let Some(ch) = self.next_char() {
                        if ch == '\n' {
                            break;
                        }
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, VdfParseError> {
        self.skip_whitespace_and_comments();
        let Some(c) = self.peek_char() else {
            return Ok(None);
        };

        if c == '{' {
            self.next_char();
            return Ok(Some(Token::OpenBrace));
        }
        if c == '}' {
            self.next_char();
            return Ok(Some(Token::CloseBrace));
        }

        if c == '"' {
            self.next_char();
            let mut s = String::new();
            let start_line = self.line;
            let mut escaped = false;
            while let Some(ch) = self.next_char() {
                if escaped {
                    match ch {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        other => {
                            s.push('\\');
                            s.push(other);
                        }
                    }
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    return Ok(Some(Token::String(s)));
                } else {
                    s.push(ch);
                }
            }
            return Err(VdfParseError {
                message: "unterminated quoted string".to_string(),
                line: start_line,
            });
        }

        // Bare string
        let mut s = String::new();
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() || ch == '{' || ch == '}' || ch == '"' {
                break;
            }
            if ch == '/' && self.pos + 1 < self.chars.len() && self.chars[self.pos + 1].1 == '/' {
                break;
            }
            s.push(ch);
            self.next_char();
        }

        if s.is_empty() {
            Err(VdfParseError {
                message: format!("unexpected character '{}'", c),
                line: self.line,
            })
        } else {
            Ok(Some(Token::String(s)))
        }
    }
}

/// Steam's own files nest a handful of levels; anything deeper is corrupt, and
/// would otherwise overflow the stack.
const MAX_DEPTH: usize = 64;

pub fn parse_vdf_text(input: &str) -> Result<Vec<(String, VdfValue)>, VdfParseError> {
    let mut lexer = VdfLexer::new(input);
    let mut root = Vec::new();

    while let Some(token) = lexer.next_token()? {
        match token {
            Token::String(key) => {
                let val = parse_value(&mut lexer, 0)?;
                root.push((key, val));
            }
            Token::CloseBrace => {
                return Err(VdfParseError {
                    message: "unexpected '}' at top level".to_string(),
                    line: lexer.line,
                });
            }
            Token::OpenBrace => {
                return Err(VdfParseError {
                    message: "unexpected '{' without key at top level".to_string(),
                    line: lexer.line,
                });
            }
        }
    }

    Ok(root)
}

fn parse_value(lexer: &mut VdfLexer<'_>, depth: usize) -> Result<VdfValue, VdfParseError> {
    if depth > MAX_DEPTH {
        return Err(VdfParseError {
            message: format!("nested deeper than {MAX_DEPTH} levels"),
            line: lexer.line,
        });
    }
    let token = lexer.next_token()?.ok_or_else(|| VdfParseError {
        message: "unexpected end of input, expected value or '{'".to_string(),
        line: lexer.line,
    })?;

    match token {
        Token::String(val) => Ok(VdfValue::String(val)),
        Token::OpenBrace => {
            let mut obj = Vec::new();
            loop {
                let tok = lexer.next_token()?.ok_or_else(|| VdfParseError {
                    message: "unexpected end of input in object, expected '}'".to_string(),
                    line: lexer.line,
                })?;
                match tok {
                    Token::CloseBrace => break,
                    Token::String(key) => {
                        let val = parse_value(lexer, depth + 1)?;
                        obj.push((key, val));
                    }
                    Token::OpenBrace => {
                        return Err(VdfParseError {
                            message: "unexpected '{' without key in object".to_string(),
                            line: lexer.line,
                        });
                    }
                }
            }
            Ok(VdfValue::Object(obj))
        }
        Token::CloseBrace => Err(VdfParseError {
            message: "unexpected '}' where value expected".to_string(),
            line: lexer.line,
        }),
    }
}
