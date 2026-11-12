//! Canonical cross-surface fault corpus.
//!
//! ONE table — (fault behavior → owning core discriminant → CLI witness) —
//! for the six fault behaviors pinned on both surfaces: `tests/core`
//! (`error_api_matrix`, lib discriminants) and `tests/cli`
//! (`error_api_pass{1,2,3,4}`, exit codes + envelopes). The pass/matrix files
//! stay thin witnesses: they keep their own triggers and assertions, while
//! the cross-surface agreement target
//! (`tests/cli/fault_corpus_agreement.rs`) reads its expectations from
//! [`FAULT_CORPUS`]. A contract change edits one row here, not two suites.
//!
//! Discriminant encoding is [`crate::store_error_discriminant`]:
//! 0 = Database, 1 = Io, 2 = Other or EmptyIndex (fail-closed non-sqlite
//! refusals). CLI witnesses are `(exit_code, error.kind)` pairs from
//! [`crate::assert_failure_envelope`]: `(1, "usage")` or `(2, "operational")`.
//!
//! | # | behavior (`FaultBehavior`) | owner (core discriminant) | witness (CLI exit + kind) |
//! |---|----------------------------|---------------------------|-----------------------------|
//! | 1 | `GarbageIndexDb` | 0 Database (`IndexStore::open` rejects garbage) | (2, operational) — corrupt `--index-path` / corrupt db drill |
//! | 2 | `MissingRoot` | 2 Other (read layer: `Searcher::new` converts missing-root Io; write layer `Indexer::new` surfaces Io=1) | (2, operational) — search on a missing root, both modes |
//! | 3 | `OversizeQuery` | 2 Other (`Searcher::search` ingress rejects) | (2, operational) — oversize query search, both modes |
//! | 4 | `InvalidRegex` | 2 Other (`Searcher::search` rejects `regex:(unclosed`) | (2, operational) — invalid-regex search/drill, both modes |
//! | 5 | `OversizeFileFilter` | 2 Other (`Searcher::new` rejects the length gate) | (2, operational) — oversize `--file-filter` search, both modes |
//! | 6 | `DeletedIndexState` | 2 Other (`Searcher::new` fails closed on a removed db; fresh index heals) | (2, operational) — deleted `.asgrep` drill, both modes, index heals |
//!
//! No behavior change: this module only names the contract both suites
//! already pin. Message text never participates.

use std::fmt;

/// The six fault behaviors duplicated across the core and CLI error suites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaultBehavior {
    /// Non-SQLite bytes at the index path.
    GarbageIndexDb,
    /// A root directory that does not exist.
    MissingRoot,
    /// A query longer than `MAX_QUERY_CHARS`.
    OversizeQuery,
    /// A `regex:(` query that never closes.
    InvalidRegex,
    /// A `--file-filter` longer than `MAX_FILE_FILTER_CHARS`.
    OversizeFileFilter,
    /// The `.asgrep` index state removed after indexing.
    DeletedIndexState,
}

/// One corpus row: the behavior, the owning core discriminant, and the CLI
/// witness shape the agreement test holds both surfaces to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultAgreement {
    /// Which duplicated fault this row owns.
    pub behavior: FaultBehavior,
    /// [`crate::store_error_discriminant`] the core read layer must report.
    pub core_discriminant: u8,
    /// Process exit the CLI must report (`assert_failure_envelope`).
    pub cli_exit: i32,
    /// `error.kind` the CLI machine envelope must carry.
    pub cli_kind: &'static str,
}

impl fmt::Display for FaultBehavior {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            FaultBehavior::GarbageIndexDb => "garbage-index-db",
            FaultBehavior::MissingRoot => "missing-root",
            FaultBehavior::OversizeQuery => "oversize-query",
            FaultBehavior::InvalidRegex => "invalid-regex",
            FaultBehavior::OversizeFileFilter => "oversize-file-filter",
            FaultBehavior::DeletedIndexState => "deleted-index-state",
        };
        f.write_str(name)
    }
}

/// The canonical table: exactly one row per duplicated fault behavior.
pub const FAULT_CORPUS: [FaultAgreement; 6] = [
    FaultAgreement {
        behavior: FaultBehavior::GarbageIndexDb,
        core_discriminant: 0,
        cli_exit: 2,
        cli_kind: "operational",
    },
    FaultAgreement {
        behavior: FaultBehavior::MissingRoot,
        core_discriminant: 2,
        cli_exit: 2,
        cli_kind: "operational",
    },
    FaultAgreement {
        behavior: FaultBehavior::OversizeQuery,
        core_discriminant: 2,
        cli_exit: 2,
        cli_kind: "operational",
    },
    FaultAgreement {
        behavior: FaultBehavior::InvalidRegex,
        core_discriminant: 2,
        cli_exit: 2,
        cli_kind: "operational",
    },
    FaultAgreement {
        behavior: FaultBehavior::OversizeFileFilter,
        core_discriminant: 2,
        cli_exit: 2,
        cli_kind: "operational",
    },
    FaultAgreement {
        behavior: FaultBehavior::DeletedIndexState,
        core_discriminant: 2,
        cli_exit: 2,
        cli_kind: "operational",
    },
];

/// Look up the agreement row for a behavior. Panics on a missing row: an
/// unwitnessed corpus variant is a test failure, not a fallible lookup.
pub fn agreement(behavior: FaultBehavior) -> FaultAgreement {
    FAULT_CORPUS
        .iter()
        .find(|row| row.behavior == behavior)
        .copied()
        .unwrap_or_else(|| panic!("fault corpus has no row for {behavior}"))
}
