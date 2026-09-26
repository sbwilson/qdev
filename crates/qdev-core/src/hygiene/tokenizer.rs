//! Language-aware lexical comment tokenizers for Rust, Swift, and Python.
//!
//! Correctly parses line comments, block comments (with nesting), and doc comments,
//! while shielding comments inside standard, multiline, raw strings and character literals.

use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentKind {
    Line,
    Block,
    DocLine,
    DocBlock,
}

impl CommentKind {
    pub fn is_doc(&self) -> bool {
        matches!(self, CommentKind::DocLine | CommentKind::DocBlock)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentLine {
    pub line_number: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentToken {
    pub kind: CommentKind,
    pub start_line: usize,
    pub end_line: usize,
    pub lines: Vec<CommentLine>,
    pub is_trailing: bool,
}

fn is_trailing_on_line(chars: &[char], start_idx: usize) -> bool {
    let mut j = start_idx;
    while j > 0 {
        j -= 1;
        if chars[j] == '\n' {
            return false;
        }
        if !chars[j].is_whitespace() {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
    Rust,
    Swift,
    Python,
}

impl SupportedLanguage {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_lowercase();
        match ext.as_str() {
            "rs" => Some(SupportedLanguage::Rust),
            "swift" => Some(SupportedLanguage::Swift),
            "py" => Some(SupportedLanguage::Python),
            _ => None,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "rust" | "rs" => Some(SupportedLanguage::Rust),
            "swift" => Some(SupportedLanguage::Swift),
            "python" | "py" => Some(SupportedLanguage::Python),
            _ => None,
        }
    }
}

/// Tokenizes all comments in a source string according to the language's lexical rules.
pub fn tokenize_comments(source: &str, lang: SupportedLanguage) -> Vec<CommentToken> {
    match lang {
        SupportedLanguage::Rust => tokenize_rust(source),
        SupportedLanguage::Swift => tokenize_swift(source),
        SupportedLanguage::Python => tokenize_python(source),
    }
}

fn split_comment_lines(text: &str, start_line: usize) -> Vec<CommentLine> {
    text.split('\n')
        .enumerate()
        .map(|(offset, l)| {
            let stripped = l.strip_suffix('\r').unwrap_or(l);
            CommentLine {
                line_number: start_line + offset,
                text: stripped.to_string(),
            }
        })
        .collect()
}

/// Lexical tokenizer for Rust (.rs).
fn tokenize_rust(source: &str) -> Vec<CommentToken> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut current_line = 1;

    while i < n {
        // Line comment: //
        if chars[i] == '/' && i + 1 < n && chars[i + 1] == '/' {
            let start_line = current_line;
            let start_idx = i;
            let kind = if i + 2 < n && chars[i + 2] == '/' {
                if i + 3 < n && chars[i + 3] == '/' {
                    CommentKind::Line // 4 or more slashes treated as normal line comment
                } else {
                    CommentKind::DocLine // ///
                }
            } else if i + 2 < n && chars[i + 2] == '!' {
                CommentKind::DocLine // //!
            } else {
                CommentKind::Line
            };

            // Scan to end of line
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            let raw_text: String = chars[start_idx..i].iter().collect();
            let stripped = raw_text.strip_suffix('\r').unwrap_or(&raw_text);
            let is_trailing = is_trailing_on_line(&chars, start_idx);
            tokens.push(CommentToken {
                kind,
                start_line,
                end_line: start_line,
                lines: vec![CommentLine {
                    line_number: start_line,
                    text: stripped.to_string(),
                }],
                is_trailing,
            });
            continue;
        }

        // Block comment: /*
        if chars[i] == '/' && i + 1 < n && chars[i + 1] == '*' {
            let start_line = current_line;
            let start_idx = i;

            let is_doc = if i + 2 < n && chars[i + 2] == '*' {
                // /** is doc unless followed by * or / (e.g. /*** or /**/)
                !(i + 3 < n && (chars[i + 3] == '*' || chars[i + 3] == '/'))
            } else if i + 2 < n && chars[i + 2] == '!' {
                true // /*!
            } else {
                false
            };
            let kind = if is_doc {
                CommentKind::DocBlock
            } else {
                CommentKind::Block
            };

            let mut depth = 1;
            i += 2;
            while i < n && depth > 0 {
                if chars[i] == '\n' {
                    current_line += 1;
                    i += 1;
                } else if chars[i] == '/' && i + 1 < n && chars[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && i + 1 < n && chars[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let end_line = current_line;
            let raw_text: String = chars[start_idx..i].iter().collect();
            let lines = split_comment_lines(&raw_text, start_line);
            let is_trailing = is_trailing_on_line(&chars, start_idx);
            tokens.push(CommentToken {
                kind,
                start_line,
                end_line,
                lines,
                is_trailing,
            });
            continue;
        }

        // Quoted string literal: "..."
        if chars[i] == '"' {
            i += 1;
            while i < n {
                if chars[i] == '\\' {
                    i += 1;
                    if i < n {
                        if chars[i] == '\n' {
                            current_line += 1;
                        }
                        i += 1;
                    }
                } else if chars[i] == '"' {
                    i += 1;
                    break;
                } else {
                    if chars[i] == '\n' {
                        current_line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }

        // Raw string literal: r"...", r#"..."#, r##"..."##
        if chars[i] == 'r' {
            let mut hash_count = 0;
            let mut k = i + 1;
            while k < n && chars[k] == '#' {
                hash_count += 1;
                k += 1;
            }
            if k < n && chars[k] == '"' {
                i = k + 1; // start inside raw string
                while i < n {
                    if chars[i] == '\n' {
                        current_line += 1;
                        i += 1;
                    } else if chars[i] == '"' {
                        let mut matches_hashes = true;
                        for h in 0..hash_count {
                            if i + 1 + h >= n || chars[i + 1 + h] != '#' {
                                matches_hashes = false;
                                break;
                            }
                        }
                        if matches_hashes {
                            i += 1 + hash_count;
                            break;
                        } else {
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            i += 1;
            continue;
        }

        // Character literal: '...' (careful with lifetimes like 'a)
        if chars[i] == '\'' {
            // Check if char literal
            let is_char = if i + 1 < n && chars[i + 1] == '\\' {
                // Escaped char literal: '\'', '\\', '\n', '\x7F', '\u{1234}'
                // If escaped char is '\'', closing quote is at i + 3
                let start_j = if i + 2 < n && chars[i + 2] == '\'' {
                    i + 3
                } else {
                    i + 2
                };
                let mut found_close = false;
                let mut j = start_j;
                while j < n && chars[j] != '\n' && j - i <= 10 {
                    if chars[j] == '\'' {
                        found_close = true;
                        i = j + 1;
                        break;
                    }
                    j += 1;
                }
                found_close
            } else if i + 2 < n && chars[i + 2] == '\'' && chars[i + 1] != '\n' && chars[i + 1] != '\\' {
                // Simple single char literal: 'x', '/', '*'
                i += 3;
                true
            } else {
                false
            };

            if is_char {
                continue;
            }

            // Otherwise lifetime or syntax quote, advance 1
            i += 1;
            continue;
        }

        if chars[i] == '\n' {
            current_line += 1;
        }
        i += 1;
    }

    tokens
}

fn scan_swift_interpolation(chars: &[char], n: usize, i: &mut usize, current_line: &mut usize) {
    // *i is at '\\', chars[*i + 1] is '('
    *i += 2;
    let mut paren_depth = 1;
    while *i < n && paren_depth > 0 {
        match chars[*i] {
            '\n' => {
                *current_line += 1;
                *i += 1;
            }
            '\\' => {
                *i += 1;
                if *i < n {
                    if chars[*i] == '\n' {
                        *current_line += 1;
                    }
                    *i += 1;
                }
            }
            '"' => {
                // Nested string literal inside interpolation
                if *i + 2 < n && chars[*i + 1] == '"' && chars[*i + 2] == '"' {
                    *i += 3;
                    while *i < n {
                        if chars[*i] == '\\' {
                            *i += 1;
                            if *i < n {
                                if chars[*i] == '\n' {
                                    *current_line += 1;
                                }
                                *i += 1;
                            }
                        } else if chars[*i] == '"' && *i + 2 < n && chars[*i + 1] == '"' && chars[*i + 2] == '"' {
                            *i += 3;
                            break;
                        } else {
                            if chars[*i] == '\n' {
                                *current_line += 1;
                            }
                            *i += 1;
                        }
                    }
                } else {
                    *i += 1;
                    while *i < n {
                        if chars[*i] == '\\' {
                            *i += 1;
                            if *i < n {
                                if chars[*i] == '\n' {
                                    *current_line += 1;
                                }
                                *i += 1;
                            }
                        } else if chars[*i] == '"' {
                            *i += 1;
                            break;
                        } else {
                            if chars[*i] == '\n' {
                                *current_line += 1;
                            }
                            *i += 1;
                        }
                    }
                }
            }
            '(' => {
                paren_depth += 1;
                *i += 1;
            }
            ')' => {
                paren_depth -= 1;
                *i += 1;
            }
            _ => {
                *i += 1;
            }
        }
    }
}

/// Lexical tokenizer for Swift (.swift).
fn tokenize_swift(source: &str) -> Vec<CommentToken> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut current_line = 1;

    while i < n {
        // Line comment: //
        if chars[i] == '/' && i + 1 < n && chars[i + 1] == '/' {
            let start_line = current_line;
            let start_idx = i;
            let kind = if i + 2 < n && chars[i + 2] == '/' && !(i + 3 < n && chars[i + 3] == '/') {
                CommentKind::DocLine // ///
            } else {
                CommentKind::Line
            };

            while i < n && chars[i] != '\n' {
                i += 1;
            }
            let raw_text: String = chars[start_idx..i].iter().collect();
            let stripped = raw_text.strip_suffix('\r').unwrap_or(&raw_text);
            let is_trailing = is_trailing_on_line(&chars, start_idx);
            tokens.push(CommentToken {
                kind,
                start_line,
                end_line: start_line,
                lines: vec![CommentLine {
                    line_number: start_line,
                    text: stripped.to_string(),
                }],
                is_trailing,
            });
            continue;
        }

        // Block comment: /*
        if chars[i] == '/' && i + 1 < n && chars[i + 1] == '*' {
            let start_line = current_line;
            let start_idx = i;
            let is_doc = i + 2 < n && chars[i + 2] == '*' && !(i + 3 < n && (chars[i + 3] == '*' || chars[i + 3] == '/'));
            let kind = if is_doc {
                CommentKind::DocBlock
            } else {
                CommentKind::Block
            };

            let mut depth = 1;
            i += 2;
            while i < n && depth > 0 {
                if chars[i] == '\n' {
                    current_line += 1;
                    i += 1;
                } else if chars[i] == '/' && i + 1 < n && chars[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && i + 1 < n && chars[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let end_line = current_line;
            let raw_text: String = chars[start_idx..i].iter().collect();
            let lines = split_comment_lines(&raw_text, start_line);
            let is_trailing = is_trailing_on_line(&chars, start_idx);
            tokens.push(CommentToken {
                kind,
                start_line,
                end_line,
                lines,
                is_trailing,
            });
            continue;
        }

        // Swift Raw String literals: #"..."#, ##"..."##, #"""..."""#
        if chars[i] == '#' {
            let mut hash_count = 0;
            let mut k = i;
            while k < n && chars[k] == '#' {
                hash_count += 1;
                k += 1;
            }
            if k < n && chars[k] == '"' {
                // Check if multiline raw string #"""
                let is_multiline = k + 2 < n && chars[k + 1] == '"' && chars[k + 2] == '"';
                if is_multiline {
                    i = k + 3;
                    while i < n {
                        if chars[i] == '\n' {
                            current_line += 1;
                            i += 1;
                        } else if chars[i] == '"' && i + 2 < n && chars[i + 1] == '"' && chars[i + 2] == '"' {
                            let mut matches_hashes = true;
                            for h in 0..hash_count {
                                if i + 3 + h >= n || chars[i + 3 + h] != '#' {
                                    matches_hashes = false;
                                    break;
                                }
                            }
                            if matches_hashes {
                                i += 3 + hash_count;
                                break;
                            } else {
                                i += 1;
                            }
                        } else {
                            i += 1;
                        }
                    }
                    continue;
                } else {
                    i = k + 1;
                    while i < n {
                        if chars[i] == '\n' {
                            current_line += 1;
                            i += 1;
                        } else if chars[i] == '"' {
                            let mut matches_hashes = true;
                            for h in 0..hash_count {
                                if i + 1 + h >= n || chars[i + 1 + h] != '#' {
                                    matches_hashes = false;
                                    break;
                                }
                            }
                            if matches_hashes {
                                i += 1 + hash_count;
                                break;
                            } else {
                                i += 1;
                            }
                        } else {
                            i += 1;
                        }
                    }
                    continue;
                }
            }
            i += 1;
            continue;
        }

        // Swift Multiline String literal: """
        if chars[i] == '"' && i + 2 < n && chars[i + 1] == '"' && chars[i + 2] == '"' {
            i += 3;
            while i < n {
                if chars[i] == '\\' && i + 1 < n && chars[i + 1] == '(' {
                    scan_swift_interpolation(&chars, n, &mut i, &mut current_line);
                } else if chars[i] == '\\' {
                    i += 1;
                    if i < n {
                        if chars[i] == '\n' {
                            current_line += 1;
                        }
                        i += 1;
                    }
                } else if chars[i] == '"' && i + 2 < n && chars[i + 1] == '"' && chars[i + 2] == '"' {
                    i += 3;
                    break;
                } else {
                    if chars[i] == '\n' {
                        current_line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }

        // Swift standard String literal: "..."
        if chars[i] == '"' {
            i += 1;
            while i < n {
                if chars[i] == '\\' && i + 1 < n && chars[i + 1] == '(' {
                    scan_swift_interpolation(&chars, n, &mut i, &mut current_line);
                } else if chars[i] == '\\' {
                    i += 1;
                    if i < n {
                        if chars[i] == '\n' {
                            current_line += 1;
                        }
                        i += 1;
                    }
                } else if chars[i] == '"' {
                    i += 1;
                    break;
                } else {
                    if chars[i] == '\n' {
                        current_line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }

        if chars[i] == '\n' {
            current_line += 1;
        }
        i += 1;
    }

    tokens
}

/// Lexical tokenizer for Python (.py).
fn tokenize_python(source: &str) -> Vec<CommentToken> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut current_line = 1;

    while i < n {
        // Triple-double-quoted string: """...""" (treated as doc comment per spec)
        if chars[i] == '"' && i + 2 < n && chars[i + 1] == '"' && chars[i + 2] == '"' {
            let start_line = current_line;
            let start_idx = i;
            i += 3;
            while i < n {
                if chars[i] == '\\' {
                    i += 1;
                    if i < n {
                        if chars[i] == '\n' {
                            current_line += 1;
                        }
                        i += 1;
                    }
                } else if chars[i] == '"' && i + 2 < n && chars[i + 1] == '"' && chars[i + 2] == '"' {
                    i += 3;
                    break;
                } else {
                    if chars[i] == '\n' {
                        current_line += 1;
                    }
                    i += 1;
                }
            }
            let end_line = current_line;
            let raw_text: String = chars[start_idx..i].iter().collect();
            let lines = split_comment_lines(&raw_text, start_line);
            let is_trailing = is_trailing_on_line(&chars, start_idx);
            tokens.push(CommentToken {
                kind: CommentKind::DocBlock,
                start_line,
                end_line,
                lines,
                is_trailing,
            });
            continue;
        }

        // Triple-single-quoted string: '''...''' (treated as doc comment per spec)
        if chars[i] == '\'' && i + 2 < n && chars[i + 1] == '\'' && chars[i + 2] == '\'' {
            let start_line = current_line;
            let start_idx = i;
            i += 3;
            while i < n {
                if chars[i] == '\\' {
                    i += 1;
                    if i < n {
                        if chars[i] == '\n' {
                            current_line += 1;
                        }
                        i += 1;
                    }
                } else if chars[i] == '\'' && i + 2 < n && chars[i + 1] == '\'' && chars[i + 2] == '\'' {
                    i += 3;
                    break;
                } else {
                    if chars[i] == '\n' {
                        current_line += 1;
                    }
                    i += 1;
                }
            }
            let end_line = current_line;
            let raw_text: String = chars[start_idx..i].iter().collect();
            let lines = split_comment_lines(&raw_text, start_line);
            let is_trailing = is_trailing_on_line(&chars, start_idx);
            tokens.push(CommentToken {
                kind: CommentKind::DocBlock,
                start_line,
                end_line,
                lines,
                is_trailing,
            });
            continue;
        }

        // Standard double-quoted string: "..."
        if chars[i] == '"' {
            i += 1;
            while i < n {
                if chars[i] == '\\' {
                    i += 1;
                    if i < n {
                        if chars[i] == '\n' {
                            current_line += 1;
                        }
                        i += 1;
                    }
                } else if chars[i] == '"' {
                    i += 1;
                    break;
                } else if chars[i] == '\n' {
                    current_line += 1;
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        // Standard single-quoted string: '...'
        if chars[i] == '\'' {
            i += 1;
            while i < n {
                if chars[i] == '\\' {
                    i += 1;
                    if i < n {
                        if chars[i] == '\n' {
                            current_line += 1;
                        }
                        i += 1;
                    }
                } else if chars[i] == '\'' {
                    i += 1;
                    break;
                } else if chars[i] == '\n' {
                    current_line += 1;
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        // Line comment: #
        if chars[i] == '#' {
            let start_line = current_line;
            let start_idx = i;
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            let raw_text: String = chars[start_idx..i].iter().collect();
            let stripped = raw_text.strip_suffix('\r').unwrap_or(&raw_text);
            let is_trailing = is_trailing_on_line(&chars, start_idx);
            tokens.push(CommentToken {
                kind: CommentKind::Line,
                start_line,
                end_line: start_line,
                lines: vec![CommentLine {
                    line_number: start_line,
                    text: stripped.to_string(),
                }],
                is_trailing,
            });
            continue;
        }

        if chars[i] == '\n' {
            current_line += 1;
        }
        i += 1;
    }

    tokens
}
