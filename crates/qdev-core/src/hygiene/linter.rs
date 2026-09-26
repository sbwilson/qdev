//! Hygiene comment linter rules engine.
//!
//! Enforces:
//! - `max_inline_comment_lines`: non-doc inline comment blocks exceeding the limit.
//! - `forbid_patterns`: matches against configured forbidden regexes.
//! - `story_banner`: story banners (e.g. `STORY \d+`, `STORY E\d+S\d+`, `⭐ STORY`).
//! - `review_round`: review-round narratives or wave markers (`Review round \d+`, `Wave \d+`, `Pass \d+`).
//!
//! Compact citations matching `citation_pattern` are whitelisted and never flagged or counted.

use regex::Regex;

use crate::config::{HygieneConfig, DEFAULT_CITATION_PATTERN};
use crate::errors::QdevError;
use super::tokenizer::CommentToken;

pub const RULE_MAX_INLINE_COMMENT_LINES: &str = "max_inline_comment_lines";
pub const RULE_FORBID_PATTERNS: &str = "forbid_patterns";
pub const RULE_STORY_BANNER: &str = "story_banner";
pub const RULE_REVIEW_ROUND: &str = "review_round";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HygieneFinding {
    pub file: String,
    pub line: usize,
    pub location: String,
    pub rule_id: String,
    pub excerpt: String,
}

/// Linter engine for inspecting extracted comment tokens in a file.
pub struct HygieneLinter {
    citation_regex: Regex,
    story_banner_regex: Regex,
    review_round_regex: Regex,
    forbid_regexes: Vec<(String, Regex)>,
    max_inline_comment_lines: u32,
}

impl HygieneLinter {
    pub fn new(config: &HygieneConfig) -> Result<Self, QdevError> {
        let citation_pat = config
            .citation_pattern
            .as_deref()
            .unwrap_or(DEFAULT_CITATION_PATTERN);

        let citation_regex = Regex::new(citation_pat).map_err(|e| {
            QdevError::usage_error(format!("Invalid regex in hygiene.citation_pattern '{}': {}", citation_pat, e))
        })?;

        // Story/epic banner pattern: ⭐ story/epic, STORY \d+, EPIC \d+, STORY E\d+S\d+, etc.
        let story_banner_regex = Regex::new(
            r"(?i)(⭐[^\n]*(?:\bstory\b|\bepic\b)|(?:\bstory\b|\bepic\b)[\s:*\-_#]+(?:e\d+s\d+|\d+(?:\.\d+)*))",
        )
        .expect("valid story_banner regex");

        // Review round / wave / pass markers requiring review or findings context
        let review_round_regex = Regex::new(
            r"(?i)\b(review\s+round\s+\d+|round\s+\d+\s+(?:review|findings)|review\s+pass\s+\d+|round\s+\d+|wave\s+\d+)\b",
        )
        .expect("valid review_round regex");

        let mut forbid_regexes = Vec::new();
        for pat in &config.forbid_patterns {
            let re = Regex::new(pat).map_err(|e| {
                QdevError::usage_error(format!("Invalid regex in hygiene.forbid_patterns '{}': {}", pat, e))
            })?;
            forbid_regexes.push((pat.clone(), re));
        }

        Ok(Self {
            citation_regex,
            story_banner_regex,
            review_round_regex,
            forbid_regexes,
            max_inline_comment_lines: config.max_inline_comment_lines,
        })
    }

    /// Lints comments extracted from a single file, producing findings.
    pub fn lint(&self, file_path: &str, tokens: &[CommentToken]) -> Vec<HygieneFinding> {
        let mut findings = Vec::new();

        // 1. Check narrative markers and forbid_patterns on all comment lines
        for token in tokens {
            for comment_line in &token.lines {
                let trimmed = comment_line.text.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // If line contains a compact citation, it is exempt from narrative & forbid checks
                if self.citation_regex.is_match(&comment_line.text) {
                    continue;
                }

                // Check story banner
                if self.story_banner_regex.is_match(&comment_line.text) {
                    findings.push(HygieneFinding {
                        file: file_path.to_string(),
                        line: comment_line.line_number,
                        location: format!("{}:{}", file_path, comment_line.line_number),
                        rule_id: RULE_STORY_BANNER.to_string(),
                        excerpt: trimmed.to_string(),
                    });
                }

                // Check review round
                if self.review_round_regex.is_match(&comment_line.text) {
                    findings.push(HygieneFinding {
                        file: file_path.to_string(),
                        line: comment_line.line_number,
                        location: format!("{}:{}", file_path, comment_line.line_number),
                        rule_id: RULE_REVIEW_ROUND.to_string(),
                        excerpt: trimmed.to_string(),
                    });
                }

                // Check forbid patterns
                for (_pat, re) in &self.forbid_regexes {
                    if re.is_match(&comment_line.text) {
                        findings.push(HygieneFinding {
                            file: file_path.to_string(),
                            line: comment_line.line_number,
                            location: format!("{}:{}", file_path, comment_line.line_number),
                            rule_id: RULE_FORBID_PATTERNS.to_string(),
                            excerpt: trimmed.to_string(),
                        });
                        break; // One forbid_patterns finding per line
                    }
                }
            }
        }

        // 2. Check max_inline_comment_lines on contiguous non-doc comment blocks
        // Only group standalone or multiline comment tokens (ignore single-line trailing annotations on code statements)
        let non_doc_tokens: Vec<&CommentToken> = tokens
            .iter()
            .filter(|t| !t.kind.is_doc() && (!t.is_trailing || t.start_line != t.end_line))
            .collect();

        if !non_doc_tokens.is_empty() {
            let mut current_block: Vec<&CommentToken> = Vec::new();

            for &token in &non_doc_tokens {
                if current_block.is_empty() {
                    current_block.push(token);
                } else {
                    let prev_token = current_block.last().unwrap();
                    // Adjacent if starting at or before prev_end_line + 1
                    if token.start_line <= prev_token.end_line + 1 {
                        current_block.push(token);
                    } else {
                        // Evaluate completed block
                        self.evaluate_block(file_path, &current_block, &mut findings);
                        current_block.clear();
                        current_block.push(token);
                    }
                }
            }

            if !current_block.is_empty() {
                self.evaluate_block(file_path, &current_block, &mut findings);
            }
        }

        findings
    }

    fn evaluate_block(
        &self,
        file_path: &str,
        block: &[&CommentToken],
        findings: &mut Vec<HygieneFinding>,
    ) {
        if block.is_empty() {
            return;
        }

        let block_start_line = block.first().unwrap().start_line;
        let mut non_citation_line_count = 0;
        let mut first_line_excerpt = String::new();

        for token in block {
            for line in &token.lines {
                let trimmed = line.text.trim();
                if first_line_excerpt.is_empty() && !trimmed.is_empty() {
                    first_line_excerpt = trimmed.to_string();
                }

                // Lines containing citations do not count toward max_inline_comment_lines
                if !self.citation_regex.is_match(&line.text) {
                    non_citation_line_count += 1;
                }
            }
        }

        if non_citation_line_count > self.max_inline_comment_lines as usize {
            if first_line_excerpt.is_empty() {
                first_line_excerpt = block
                    .first()
                    .and_then(|t| t.lines.first())
                    .map(|l| l.text.trim().to_string())
                    .unwrap_or_default();
            }

            findings.push(HygieneFinding {
                file: file_path.to_string(),
                line: block_start_line,
                location: format!("{}:{}", file_path, block_start_line),
                rule_id: RULE_MAX_INLINE_COMMENT_LINES.to_string(),
                excerpt: first_line_excerpt,
            });
        }
    }
}

/// Convenience function to lint comments with given configuration.
pub fn lint_comments(
    file_path: &str,
    tokens: &[CommentToken],
    config: &HygieneConfig,
) -> Result<Vec<HygieneFinding>, QdevError> {
    let linter = HygieneLinter::new(config)?;
    Ok(linter.lint(file_path, tokens))
}
