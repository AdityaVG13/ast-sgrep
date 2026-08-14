//! Optional SCIP index load (kgvi.1).
//!
//! Missing or malformed input degrades; it never fails the caller. Graph
//! mutation and CLI wiring are kgvi.2 / kgvi.3. The interchange for this
//! slice is a JSON projection of SCIP documents/occurrences. Protobuf wire
//! files degrade until a later parser lands.

use crate::MAX_INDEX_FILE_BYTES;
use serde::Deserialize;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Channel name recorded on `DegradedChannel` when SCIP cannot be used.
pub const SCIP_CHANNEL: &str = "scip";

/// SCIP `SymbolRole.Definition` bit (sourcegraph/scip).
pub const SCIP_ROLE_DEFINITION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScipLoad {
    Loaded(ScipIndex),
    Degraded { reason: String },
}

impl ScipLoad {
    pub fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded(_))
    }

    pub fn degraded_reason(&self) -> Option<&str> {
        match self {
            Self::Degraded { reason } => Some(reason.as_str()),
            Self::Loaded(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct ScipIndex {
    #[serde(default)]
    pub documents: Vec<ScipDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ScipDocument {
    #[serde(default, alias = "relativePath")]
    pub relative_path: String,
    #[serde(default)]
    pub occurrences: Vec<ScipOccurrence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ScipOccurrence {
    #[serde(default)]
    pub symbol: String,
    #[serde(default, alias = "symbolRoles")]
    pub symbol_roles: u32,
    /// SCIP range: `[startLine, startCharacter, endLine, endCharacter]`
    /// or a 3-element same-line form.
    #[serde(default)]
    pub range: Vec<u32>,
}

impl ScipOccurrence {
    pub fn is_definition(&self) -> bool {
        self.symbol_roles & SCIP_ROLE_DEFINITION != 0
    }
}

/// Load a SCIP index from `path`. Missing, unreadable, oversized, binary, or
/// malformed input returns [`ScipLoad::Degraded`] rather than `Err`.
pub fn load_scip_index(path: &Path) -> ScipLoad {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return degrade(format!("scip index not found: {}", path.display()));
        }
        Err(error) => {
            return degrade(format!(
                "scip index unreadable ({}): {}",
                error,
                path.display()
            ));
        }
    };
    let mut bytes = Vec::new();
    if let Err(error) = file
        .take(MAX_INDEX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
    {
        return degrade(format!(
            "scip index read failed ({}): {}",
            error,
            path.display()
        ));
    }
    if bytes.len() as u64 > MAX_INDEX_FILE_BYTES {
        return degrade(format!(
            "scip index exceeds {MAX_INDEX_FILE_BYTES} byte cap: {}",
            path.display()
        ));
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return degrade(format!("scip index is empty: {}", path.display()));
    }
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(0);
    if bytes.get(start) != Some(&b'{') {
        return degrade(
            "scip protobuf/binary is not parsed yet; expected a JSON SCIP document".into(),
        );
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(_) => return degrade("scip index is not valid UTF-8 JSON".into()),
    };
    match serde_json::from_str::<ScipIndex>(text) {
        Ok(index) => ScipLoad::Loaded(index),
        Err(error) => degrade(format!("malformed scip JSON: {error}")),
    }
}

fn degrade(reason: String) -> ScipLoad {
    ScipLoad::Degraded { reason }
}

#[cfg(test)]
#[path = "../../../tests/unit/core/scip.rs"]
mod tests;
