//! Names sidecar: folded symbol-name and distinct-callee vocabularies for
//! exact idf df counting without SQL row streaming.
//!
//! The cold hybrid path computes two idf vectors per query (defs over
//! `symbols.name`, caller hits over `DISTINCT callers.callee`). L5 moved the
//! counting predicate into Rust, but each call still streamed the whole
//! vocabulary through the SQLite→rusqlite row boundary. This sidecar stores
//! both vocabularies pre-folded (`str::to_lowercase`, the same fold the
//! counting predicate applies) in one flat mmap-able file, written at index
//! finalize under the rename-publish protocol (`ast_sgrep_mmap` policy).
//!
//! Coherence: the header records the `index_data_version` the vocabularies
//! were read from. Every mutating index transaction bumps that version
//! inside the transaction, so a reader that verifies the recorded generation
//! against the live store can trust the populations are exact. Any mismatch
//! (stale, corrupt, absent, `:memory:` store, or
//! `ASGREP_NO_NAMES_SIDECAR=1`) falls back to the SQL streaming path with
//! identical results — the sidecar is a pure accelerator.

use crate::store::IndexStore;
use crate::Result;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const NAMES_SIDECAR_FILE: &str = "names";
const MAGIC: &[u8; 6] = b"ASNM\0\0";
const VERSION: u32 = 1;
/// magic 6 + version u32 4 + generation i64 8 + symbols u64 8 + callees u64 8.
const HEADER_SIZE: usize = 34;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Which name population a df counting question is about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NamePopulation {
    /// Every `symbols.name` row (per-row population, matching the SQL lane).
    Symbols,
    /// Distinct `callers.callee` values.
    DistinctCallees,
}

impl NamePopulation {
    pub(crate) fn sql_source_and_column(self) -> (&'static str, &'static str) {
        match self {
            NamePopulation::Symbols => ("symbols", "name"),
            NamePopulation::DistinctCallees => ("(SELECT DISTINCT callee FROM callers)", "callee"),
        }
    }
}

pub fn sidecar_path(index_db: &Path) -> PathBuf {
    index_db
        .parent()
        .map(|parent| parent.join(NAMES_SIDECAR_FILE))
        .unwrap_or_else(|| Path::new(NAMES_SIDECAR_FILE).to_path_buf())
}

/// `ASGREP_NO_NAMES_SIDECAR=1` forces the SQL lane (A/B control arm + safety
/// valve), mirroring `ASGREP_SERIAL_CHANNELS`.
pub(crate) fn disabled_by_env() -> bool {
    std::env::var_os("ASGREP_NO_NAMES_SIDECAR").is_some()
}

fn read_population(store: &IndexStore, population: NamePopulation) -> Result<Vec<String>> {
    let (source, column) = population.sql_source_and_column();
    let conn = store.connection();
    let mut stmt = conn.prepare(&format!("SELECT {column} FROM {source}"))?;
    let mut rows = stmt.query([])?;
    let mut names = Vec::new();
    while let Some(row) = rows.next()? {
        names.push(row.get::<_, String>(0)?);
    }
    Ok(names)
}

/// Write the sidecar atomically (tmp + fsync + rename) for `generation`.
/// In-memory stores have no filesystem surface and are a no-op.
pub fn write_names_sidecar(store: &IndexStore, generation: i64) -> Result<()> {
    if crate::semantic_ivf::is_in_memory_db(store.db_path()) {
        return Ok(());
    }
    let symbols = read_population(store, NamePopulation::Symbols)?;
    let callees = read_population(store, NamePopulation::DistinctCallees)?;

    let mut header = Vec::with_capacity(HEADER_SIZE);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&VERSION.to_le_bytes());
    header.extend_from_slice(&generation.to_le_bytes());
    header.extend_from_slice(&(symbols.len() as u64).to_le_bytes());
    header.extend_from_slice(&(callees.len() as u64).to_le_bytes());

    let mut symbols_offsets = Vec::with_capacity((symbols.len() + 1) * 4);
    let mut symbols_text = Vec::new();
    write_section(&mut symbols_offsets, &mut symbols_text, &symbols);
    let mut callees_offsets = Vec::with_capacity((callees.len() + 1) * 4);
    let mut callees_text = Vec::new();
    write_section(&mut callees_offsets, &mut callees_text, &callees);

    let path = sidecar_path(store.db_path());
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        NAMES_SIDECAR_FILE,
        std::process::id(),
        sequence
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&header)?;
    // Per-population self-contained sections: [offsets][text] each, so a
    // reader chains section N+1 from section N's text end.
    file.write_all(&symbols_offsets)?;
    file.write_all(&symbols_text)?;
    file.write_all(&callees_offsets)?;
    file.write_all(&callees_text)?;
    file.sync_all()?;
    drop(file);
    match crate::semantic_ivf::replace_file(&temporary, &path) {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.into())
        }
    }
}

fn write_section(offsets: &mut Vec<u8>, text: &mut Vec<u8>, names: &[String]) {
    let mut cursor: u32 = 0;
    offsets.extend_from_slice(&cursor.to_le_bytes());
    for name in names {
        // Pre-fold exactly as the counting predicate would: `to_lowercase`.
        let folded = name.to_lowercase();
        text.extend_from_slice(folded.as_bytes());
        cursor = cursor.saturating_add(folded.len() as u32);
        offsets.extend_from_slice(&cursor.to_le_bytes());
    }
}

struct Section {
    count: usize,
    /// Byte range of the u32 offset array within the mapping.
    offsets: (usize, usize),
    /// Byte range of the folded text within the mapping.
    text: (usize, usize),
}

impl Section {
    fn dfs(&self, map: &[u8], terms: &[String]) -> (Vec<u64>, u64) {
        let mut dfs = vec![0u64; terms.len()];
        let text = &map[self.text.0..self.text.1];
        let finders: Vec<memchr::memmem::Finder<'_>> = terms
            .iter()
            .map(|term| memchr::memmem::Finder::new(term.as_bytes()))
            .collect();
        for i in 0..self.count {
            let start = read_u32(map, self.offsets.0 + i * 4) as usize;
            let end = read_u32(map, self.offsets.0 + (i + 1) * 4) as usize;
            if end < start || self.text.0 + end > self.text.1 {
                continue;
            }
            let name = &text[start..end];
            // Write path only stores UTF-8; skip the from_utf8 tax on the mill
            // df sweep. Corrupt bytes still cannot match a UTF-8 query term.
            for (t, finder) in finders.iter().enumerate() {
                if finder.find(name).is_some() {
                    dfs[t] += 1;
                }
            }
        }
        (dfs, self.count as u64)
    }

    fn cooccurrences(&self, map: &[u8], terms: &[String]) -> Vec<u64> {
        let mut co = vec![0u64; terms.len()];
        if terms.len() < 2 {
            return co;
        }
        let text = &map[self.text.0..self.text.1];
        let finders: Vec<memchr::memmem::Finder<'_>> = terms
            .iter()
            .map(|term| memchr::memmem::Finder::new(term.as_bytes()))
            .collect();
        let mut hits = vec![false; terms.len()];
        for i in 0..self.count {
            let start = read_u32(map, self.offsets.0 + i * 4) as usize;
            let end = read_u32(map, self.offsets.0 + (i + 1) * 4) as usize;
            if end < start || self.text.0 + end > self.text.1 {
                continue;
            }
            let name = &text[start..end];
            hits.fill(false);
            let mut count = 0usize;
            for (t, finder) in finders.iter().enumerate() {
                if finder.find(name).is_some() {
                    hits[t] = true;
                    count += 1;
                }
            }
            if count >= 2 {
                for (t, hit) in hits.iter().enumerate() {
                    if *hit {
                        co[t] += 1;
                    }
                }
            }
        }
        co
    }
}

fn read_u32(map: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([map[at], map[at + 1], map[at + 2], map[at + 3]])
}

/// An opened, validated names sidecar mapping.
pub struct NamesSidecar {
    map: ast_sgrep_mmap::Mmap,
    generation: i64,
    symbols: Section,
    callees: Section,
}

impl NamesSidecar {
    /// Open and validate. `None` when absent, unreadable, or malformed —
    /// callers take the SQL fallback; a malformed file is never an error.
    pub fn open(index_db: &Path) -> Option<Self> {
        if crate::semantic_ivf::is_in_memory_db(index_db) {
            return None;
        }
        let spath = sidecar_path(index_db);
        let file = match File::open(&spath) {
            Ok(f) => f,
            Err(e) => {
                return None;
            }
        };
        let map = match ast_sgrep_mmap::map_readonly(&file) {
            Ok(m) => m,
            Err(e) => {
                return None;
            }
        };
        let bytes = &map[..];
        if bytes.len() < HEADER_SIZE || &bytes[0..6] != MAGIC {
            return None;
        }
        let version = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        if version != VERSION {
            return None;
        }
        let generation = i64::from_le_bytes([
            bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17],
        ]);
        let symbols_count = u64::from_le_bytes([
            bytes[18], bytes[19], bytes[20], bytes[21], bytes[22], bytes[23], bytes[24], bytes[25],
        ]);
        let callees_count = u64::from_le_bytes([
            bytes[26], bytes[27], bytes[28], bytes[29], bytes[30], bytes[31], bytes[32], bytes[33],
        ]);
        let section = |count: u64, offsets_start: usize| -> Option<Section> {
            let count = usize::try_from(count).ok()?;
            let offsets_len = count.checked_add(1)?.checked_mul(4)?;
            let offsets_end = offsets_start.checked_add(offsets_len)?;
            let text_start = offsets_end;
            // Validate against the mapping length; the final offset must land
            // exactly at the section text length.
            let text_len = read_u32(bytes, offsets_end.checked_sub(4)?) as usize;
            let text_end = text_start.checked_add(text_len)?;
            if text_end > bytes.len() {
                return None;
            }
            Some(Section {
                count,
                offsets: (offsets_start, offsets_end),
                text: (text_start, text_end),
            })
        };
        let symbols = section(symbols_count, HEADER_SIZE)?;
        let callees = section(callees_count, symbols.text.1)?;
        // Total layout must consume the file exactly.
        if callees.text.1 != bytes.len() {
            return None;
        }
        Some(Self {
            map,
            generation,
            symbols,
            callees,
        })
    }

    pub fn generation(&self) -> i64 {
        self.generation
    }

    /// Exact-generation freshness. Any mutation bumps `index_data_version`
    /// inside its transaction, so equality is the trust boundary.
    pub fn is_fresh(&self, current_generation: i64) -> bool {
        self.generation == current_generation
    }

    /// Per-term document frequencies plus the population total, counted over
    /// the folded vocabulary with plain substring containment.
    pub fn dfs(&self, population: NamePopulation, terms: &[String]) -> (Vec<u64>, u64) {
        let section = match population {
            NamePopulation::Symbols => &self.symbols,
            NamePopulation::DistinctCallees => &self.callees,
        };
        section.dfs(&self.map, terms)
    }

    /// Per-term co-occurrence counts: names containing this term AND at least
    /// one other query term. Same folded substring predicate as [`Self::dfs`].
    pub fn cooccurrences(&self, population: NamePopulation, terms: &[String]) -> Vec<u64> {
        let section = match population {
            NamePopulation::Symbols => &self.symbols,
            NamePopulation::DistinctCallees => &self.callees,
        };
        section.cooccurrences(&self.map, terms)
    }
}

/// Store-facing entry: sidecar dfs when enabled, fresh, and openable;
/// `None` means "take the SQL lane".
pub(crate) fn sidecar_dfs_for_store(
    store: &IndexStore,
    population: NamePopulation,
    terms: &[String],
) -> Option<(Vec<u64>, u64)> {
    Some(open_sidecar_for_store(store)?.dfs(population, terms))
}

pub(crate) fn sidecar_cooccurrences_for_store(
    store: &IndexStore,
    population: NamePopulation,
    terms: &[String],
) -> Option<Vec<u64>> {
    Some(open_sidecar_for_store(store)?.cooccurrences(population, terms))
}

fn open_sidecar_for_store(store: &IndexStore) -> Option<Arc<NamesSidecar>> {
    if disabled_by_env() {
        return None;
    }
    let generation = store.index_data_version().ok()?;
    let mut slot = store.names_sidecar_slot();
    if let Some(open) = slot.as_ref() {
        if open.is_fresh(generation) {
            return Some(Arc::clone(open));
        }
    }
    let opened = Arc::new(NamesSidecar::open(store.db_path())?);
    if !opened.is_fresh(generation) {
        *slot = None;
        return None;
    }
    *slot = Some(Arc::clone(&opened));
    Some(opened)
}
