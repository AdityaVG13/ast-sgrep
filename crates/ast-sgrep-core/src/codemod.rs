//! Indexed, in-process structural codemod planning and transactional apply.
//!
//! Phases: [`plan`] builds the deterministic edit plan (read-only);
//! [`rewrite`] holds the span/template helpers planning uses; [`guard`]
//! holds the confinement + writability predicates both phases share;
//! [`apply`] stages and swaps the plan as one all-or-nothing transaction.

pub mod apply;
pub mod guard;
mod plan;
mod rewrite;

pub use apply::{apply_codemod, concurrent_capture_error};
pub use plan::plan_codemod;

use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct CodemodEdit {
    pub path: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: u32,
    pub line_end: u32,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodemodFilePlan {
    pub path: String,
    pub edits: Vec<CodemodEdit>,
    #[serde(skip)]
    pub original: String,
    #[serde(skip)]
    pub rewritten: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodemodPlan {
    pub pattern: String,
    pub rewrite: String,
    pub files_changed: usize,
    pub edit_count: usize,
    pub files: Vec<CodemodFilePlan>,
    /// Planned-for files whose owner-write bit is clear, refused
    /// reference-agreed (the reference's update-all skips them with
    /// `Cannot rewrite file … Permission denied` and exits 6) instead of
    /// being rewritten through the staged temp+rename commit, which only
    /// needs the writable PARENT directory and would otherwise silently
    /// swap a `chmod 444` file's content while keeping its read-only mode.
    /// Serialized so the dry-run preview shows exactly what apply will do.
    pub read_only_refused: Vec<String>,
    #[serde(skip)]
    pub root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodemodApplyResult {
    pub files_changed: usize,
    pub edits_applied: usize,
}

impl CodemodPlan {
    pub fn changed_paths(&self) -> Vec<PathBuf> {
        self.files
            .iter()
            .map(|file| self.root.join(&file.path))
            .collect()
    }
}
