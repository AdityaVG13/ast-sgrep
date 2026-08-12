//! Symbol identity + resolution tiers (dvc4).
//!
//! Call edges keep the last chain id + nearest enclosing fn, so name-only
//! collisions are common. This module records *how* an edge was resolved so
//! name-only guesses are never serialized as precise.

/// Stable symbol identity (dvc4). `name` alone is not identity; optional
/// fields fill in when known without pretending to be compiler-grade.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SymbolId {
    /// Source language, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Package or crate, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Module path, usually derived from the file path.
    pub module: String,
    /// Enclosing scopes, outermost first: impl block, class, namespace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owner_path: Vec<String>,
    /// The bare identifier.
    pub name: String,
}

impl SymbolId {
    pub fn new(module: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            language: None,
            package: None,
            module: module.into(),
            owner_path: Vec::new(),
            name: name.into(),
        }
    }

    pub fn with_owner(mut self, owner: impl Into<String>) -> Self {
        self.owner_path.push(owner.into());
        self
    }

    pub fn with_language(mut self, language: Option<String>) -> Self {
        self.language = language;
        self
    }

    /// Fully qualified display form, e.g. `src/auth.rs::Session::refresh`.
    pub fn qualified(&self) -> String {
        let mut parts = vec![self.module.clone()];
        parts.extend(self.owner_path.iter().cloned());
        parts.push(self.name.clone());
        parts.join("::")
    }
}

/// Resolution confidence (dvc4), strongest → weakest. Callers gate precision on this.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// A compiler or type checker resolved this.
    CompilerExact,
    /// An SCIP index resolved this.
    ScipExact,
    /// Resolved through an explicit import.
    ImportResolved,
    /// Exactly one candidate with this name in the same file.
    FileLocalUnique,
    /// Exactly one candidate with this name in the repository.
    RepositoryUnique,
    /// Matched on name alone, with no disambiguation. A guess.
    NameOnly,
    /// Several candidates match and none could be chosen.
    Ambiguous { candidates: Vec<SymbolId> },
}

impl Resolution {
    /// Honesty gate: only precise tiers may be presented as exact edges.
    pub fn is_precise(&self) -> bool {
        matches!(
            self,
            Self::CompilerExact | Self::ScipExact | Self::ImportResolved | Self::FileLocalUnique
        )
    }

    /// Stable wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CompilerExact => "compiler_exact",
            Self::ScipExact => "scip_exact",
            Self::ImportResolved => "import_resolved",
            Self::FileLocalUnique => "file_local_unique",
            Self::RepositoryUnique => "repository_unique",
            Self::NameOnly => "name_only",
            Self::Ambiguous { .. } => "ambiguous",
        }
    }

    /// Strength ordering, strongest first (0 is strongest).
    pub fn rank(&self) -> u8 {
        match self {
            Self::CompilerExact => 0,
            Self::ScipExact => 1,
            Self::ImportResolved => 2,
            Self::FileLocalUnique => 3,
            Self::RepositoryUnique => 4,
            Self::NameOnly => 5,
            Self::Ambiguous { .. } => 6,
        }
    }

    /// Classify a name match from the candidates that carry that name (dvc4).
    ///
    /// `same_file_candidates` counts definitions of the name inside the file
    /// that referenced it; `repository_candidates` counts them repository-wide.
    pub fn from_candidates(
        same_file_candidates: usize,
        repository_candidates: usize,
        candidates: impl IntoIterator<Item = SymbolId>,
    ) -> Self {
        match (same_file_candidates, repository_candidates) {
            (1, _) => Self::FileLocalUnique,
            (_, 1) => Self::RepositoryUnique,
            (_, 0) => Self::NameOnly,
            _ => {
                let collected: Vec<SymbolId> = candidates.into_iter().take(4).collect();
                if collected.len() <= 1 {
                    // Nothing to disambiguate between: this is a bare name
                    // match, not a genuine ambiguity.
                    Self::NameOnly
                } else {
                    Self::Ambiguous {
                        candidates: collected,
                    }
                }
            }
        }
    }
}

/// A graph edge with its resolution tier attached (dvc4).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedEdge {
    pub caller: SymbolId,
    pub callee: SymbolId,
    pub resolution: Resolution,
}

impl ResolvedEdge {
    /// Render for a consumer, refusing to describe a guess as exact (dvc4).
    ///
    /// Returns the edge label plus whether it is safe to treat as precise.
    pub fn describe(&self) -> (String, bool) {
        let precise = self.resolution.is_precise();
        let label = if precise {
            format!(
                "{} calls {}",
                self.caller.qualified(),
                self.callee.qualified()
            )
        } else {
            format!(
                "{} may call {} (resolved by {})",
                self.caller.qualified(),
                self.callee.name,
                self.resolution.as_str()
            )
        };
        (label, precise)
    }
}
