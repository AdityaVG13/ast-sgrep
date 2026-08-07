//! Symbol identity and edge resolution tiers (bead ast-sgrep-tef-symbol-identity-dvc4).
//!
//! Call extraction keeps the last identifier in a call chain and pairs it with
//! the nearest enclosing function name. That means `client.send()` becomes
//! `send`, same-named methods on unrelated receivers collide, overloads
//! collapse, and aliases lose identity.
//!
//! Making that resolution genuinely precise needs compiler-grade indexes. What
//! this module fixes is the part that is a correctness bug regardless: the
//! engine used to present a name-only guess with the same confidence as an
//! exact match. An edge now carries HOW it was resolved, and a name-only edge
//! must never be serialized as precise.

/// Stable identity for a symbol (dvc4).
///
/// `name` alone is not identity: two `send` methods on unrelated types are
/// different symbols. The extra components are what separate them, and each is
/// `Option` because this engine resolves them opportunistically rather than
/// pretending to have compiler knowledge it lacks.
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

/// How confidently an edge or symbol reference was resolved (dvc4).
///
/// Ordered from strongest to weakest. The ordering is meaningful: callers use
/// it to decide what may be presented as precise.
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
    Ambiguous {
        candidates: Vec<SymbolId>,
    },
}

impl Resolution {
    /// May this resolution be presented to a consumer as an exact edge?
    ///
    /// This is the honesty gate. `NameOnly` and `Ambiguous` are guesses, and a
    /// guess rendered as a fact is worse than no answer, because the reader
    /// cannot tell it needs checking.
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
