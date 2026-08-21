//! Scrubber registry for golden freezes.
//!
//! Presets live on the test path only. Product formatters must not call this.
//! `machine_contract()` replaces package `version` strings and leaves
//! `schema_version` intact.

use regex::Regex;
use std::path::Path;

/// One replace pass: regex → placeholder, or a rooted path prefix.
struct Rule {
    pattern: Regex,
    replacement: &'static str,
}

/// Ordered scrub rules applied left-to-right.
pub struct Scrubber {
    rules: Vec<Rule>,
}

impl Scrubber {
    /// Identity: no replacements.
    pub fn none() -> Self {
        Self { rules: Vec::new() }
    }

    /// Paths, UUIDs, ISO timestamps, and hex addresses.
    pub fn standard() -> Self {
        Self {
            rules: vec![
                rule(
                    r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
                    "<UUID>",
                ),
                rule(
                    r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?",
                    "<TIMESTAMP>",
                ),
                rule(r"0x[0-9a-fA-F]{6,16}", "<ADDR>"),
                rule(r"/Users/[^/\s]+", "<HOME>"),
                rule(r"/home/[^/\s]+", "<HOME>"),
                rule(r"/private/tmp", "<TMP>"),
                rule(r"/tmp", "<TMP>"),
                rule(r"[A-Za-z]:\\Users\\[^\\\s]+", "<HOME>"),
                rule(r"[A-Za-z]:\\tmp", "<TMP>"),
            ],
        }
    }

    /// [`standard`] plus package `version` fields; never `schema_version`.
    pub fn machine_contract() -> Self {
        let mut s = Self::standard();
        s.rules.push(rule(
            r#""version"\s*:\s*"[0-9]+\.[0-9]+\.[0-9]+[^"]*""#,
            r#""version": "<version>""#,
        ));
        s
    }

    /// [`standard`] plus the indexed project root → `<ROOT>`.
    pub fn search_dump(root: &Path) -> Self {
        let mut s = Self::standard();
        if let Some(raw) = root.to_str() {
            let escaped = regex::escape(raw);
            if let Ok(pattern) = Regex::new(&escaped) {
                s.rules.insert(
                    0,
                    Rule {
                        pattern,
                        replacement: "<ROOT>",
                    },
                );
            }
            let unified = raw.replace('\\', "/");
            if unified != raw {
                if let Ok(pattern) = Regex::new(&regex::escape(&unified)) {
                    s.rules.insert(
                        0,
                        Rule {
                            pattern,
                            replacement: "<ROOT>",
                        },
                    );
                }
            }
        }
        s
    }

    /// Doctor envelopes: [`standard`] only (messages stay; do not blank errors).
    pub fn doctor() -> Self {
        Self::standard()
    }

    /// Status envelopes: [`standard`] only.
    pub fn status() -> Self {
        Self::standard()
    }

    pub fn apply(&self, input: &str) -> String {
        let mut out = input.to_string();
        for rule in &self.rules {
            out = rule
                .pattern
                .replace_all(&out, rule.replacement)
                .into_owned();
        }
        out
    }
}

fn rule(pattern: &'static str, replacement: &'static str) -> Rule {
    Rule {
        pattern: Regex::new(pattern).expect("scrub regex"),
        replacement,
    }
}
