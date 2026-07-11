//! Fixture docs mention doc_only_rust and should not become code.
use std::collections::HashMap;

/// Type docs mention doc_only_rust.
pub struct GoldenWidget {
    labels: HashMap<String, String>,
}

/// Free function docs mention doc_only_rust.
pub fn top_level_helper(input: &str) -> String {
    input.to_string()
}

impl GoldenWidget {
    /// Constructor docs mention doc_only_rust.
    pub fn new(labels: HashMap<String, String>) -> Self {
        Self { labels }
    }

    /// Method docs mention doc_only_rust.
    pub fn process(&self, input: &str) -> String {
        top_level_helper(input)
    }
}

/// Enum docs mention doc_only_rust.
pub enum GoldenState {
    Ready,
    Spent,
}

/// Trait docs mention doc_only_rust.
pub trait GoldenRender {
    fn render_widget(&self) -> String;
}
