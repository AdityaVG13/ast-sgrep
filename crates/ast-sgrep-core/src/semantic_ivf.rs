use crate::semantic_ann::SemanticAnnIndex;
use crate::Result;
use ast_sgrep_mmap::{map_readonly, Mmap};
use blake3::Hasher;
use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

const MAGIC: &[u8; 6] = b"ASIVF\0";
/// v3 appends the chunk-id column after the vectors (see
/// [`save_semantic_ivf_with_ids`]). Readers accept v2 and v3: a v2 file has
/// no ids section and the embed pass falls back to SQL id resolution, so an
/// older sidecar degrades to the old path instead of failing.
const VERSION: u32 = 3;
const MIN_READER_VERSION: u32 = 2;
const HEADER_SIZE: usize = 80;
const VECTOR_ALIGNMENT: usize = 4096;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub const SEMANTIC_IVF_FILE: &str = "semantic.ivf";

/// Per-field SQLite vectors (name/docs/body/graph/tests-examples) beside concatenated primary.
/// Bump when the stored field set or packing changes so a stale sidecar cannot match.
pub const SEMANTIC_IVF_FIELD_LAYOUT: u32 = 3;

pub fn semantic_ivf_path(index_db: &Path) -> std::path::PathBuf {
    index_db
        .parent()
        .map(|parent| parent.join(SEMANTIC_IVF_FILE))
        .unwrap_or_else(|| Path::new(SEMANTIC_IVF_FILE).to_path_buf())
}

pub fn invalidate_semantic_ivf(index_db: &Path) -> Result<()> {
    // An in-memory store has NO filesystem surface.
    // `Path::new(":memory:").parent()` is `Some("")`, so the historical path
    // resolution degenerated to a CWD-relative `semantic.ivf` and the
    // legacy user_version migration deleted whatever file sat in the
    // process's working directory — recorded data loss. `:memory:` (and any
    // URI-form memory target) is a no-op here.
    if is_in_memory_db(index_db) {
        return Ok(());
    }
    let path = semantic_ivf_path(index_db);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if benign_invalidation_error(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// True for SQLite in-memory database targets (`:memory:` and the
/// `file::memory:` / `file:...?mode=memory` URI forms). Such a path can
/// never name a sidecar's directory, so invalidation is a no-op.
fn is_in_memory_db(index_db: &Path) -> bool {
    let text = index_db.to_string_lossy();
    text == ":memory:"
        || text.starts_with("file:") && (text.contains("mode=memory") || text.contains("::memory:"))
}

fn benign_invalidation_error(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::NotFound {
        return true;
    }
    #[cfg(windows)]
    {
        error.kind() == std::io::ErrorKind::PermissionDenied
            || matches!(error.raw_os_error(), Some(32) | Some(33) | Some(1224))
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn compute_ann_fingerprint(
    chunk_count: usize,
    max_chunk_id: i64,
    dim: usize,
    embed_backend: Option<&str>,
    source_generation: i64,
) -> [u8; 32] {
    fingerprint(
        chunk_count,
        max_chunk_id,
        dim,
        embed_backend,
        source_generation,
        SEMANTIC_IVF_FIELD_LAYOUT,
        None,
    )
}
/// Content-identity digest over flat vectors (additional IVF binding when vectors
/// are already loaded). Combined with data_version at build/load sites.
pub fn vectors_content_digest(vectors: &[f32]) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(b"asgrep-ivf-vectors-v1");
    h.update(&(vectors.len() as u64).to_le_bytes());
    for v in vectors {
        h.update(&v.to_le_bytes());
    }
    *h.finalize().as_bytes()
}
/// ANN fingerprint bound to vector content as well as chunk identity: catches
/// rebuilds where ids are reused but vector payloads changed.
pub fn compute_ann_fingerprint_with_content(
    chunk_count: usize,
    max_chunk_id: i64,
    dim: usize,
    embed_backend: Option<&str>,
    data_version: i64,
    content_digest: &[u8; 32],
) -> [u8; 32] {
    fingerprint(
        chunk_count,
        max_chunk_id,
        dim,
        embed_backend,
        data_version,
        SEMANTIC_IVF_FIELD_LAYOUT,
        Some(content_digest),
    )
}

fn fingerprint(
    chunk_count: usize,
    max_chunk_id: i64,
    dim: usize,
    embed_backend: Option<&str>,
    source_generation: i64,
    field_layout: u32,
    content_digest: Option<&[u8; 32]>,
) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(b"asgrep-semantic-ivf-v2");
    hasher.update(&field_layout.to_le_bytes());
    hasher.update(&(chunk_count as u64).to_le_bytes());
    hasher.update(&max_chunk_id.to_le_bytes());
    hasher.update(&(dim as u32).to_le_bytes());
    hasher.update(embed_backend.unwrap_or("semantic").as_bytes());
    // source_generation (index_data_version) disambiguates delete+re-add where
    // max_chunk_id is reused but chunk content/vectors differ (44a4 / e2hc.15).
    hasher.update(&source_generation.to_le_bytes());
    if let Some(digest) = content_digest {
        hasher.update(digest);
    }
    *hasher.finalize().as_bytes()
}

#[derive(Debug, Clone)]
struct MappedVectors {
    mmap: Arc<Mmap>,
    bytes: Range<usize>,
}

/// Trailing v3 ids section: the chunk-id column in the same order the
/// vectors were flattened (`SELECT id ... ORDER BY id`). v2 sidecars load
/// with `None` and callers fall back to SQL id resolution.
#[derive(Debug, Clone)]
struct MappedIds {
    mmap: Arc<Mmap>,
    bytes: Range<usize>,
}

impl MappedIds {
    fn as_slice(&self) -> &[i64] {
        bytemuck::try_cast_slice(&self.mmap[self.bytes.clone()])
            .expect("validated semantic IVF ids alignment")
    }
}

impl MappedVectors {
    fn as_slice(&self) -> &[f32] {
        bytemuck::try_cast_slice(&self.mmap[self.bytes.clone()])
            .expect("validated semantic IVF vector alignment")
    }

    /// Touch every page once so unique-query p90 is not a first-fault walk.
    fn prefault(&self) {
        let bytes = &self.mmap[self.bytes.clone()];
        const PAGE: usize = 4096;
        let mut offset = 0;
        while offset < bytes.len() {
            std::hint::black_box(bytes[offset]);
            offset += PAGE;
        }
        if let Some(last) = bytes.last() {
            std::hint::black_box(*last);
        }
    }
}

#[derive(Debug, Clone)]
pub struct PersistedSemanticIvf {
    pub fingerprint: [u8; 32],
    pub dim: usize,
    /// Compatibility copy for callers using the pre-1.3 public field.
    pub vectors: Vec<f32>,
    mapped_vectors: Option<MappedVectors>,
    pub index: SemanticAnnIndex,
}

impl PersistedSemanticIvf {
    pub fn from_owned(
        fingerprint: [u8; 32],
        dim: usize,
        vectors: Vec<f32>,
        index: SemanticAnnIndex,
    ) -> Self {
        Self {
            fingerprint,
            dim,
            vectors,
            mapped_vectors: None,
            index,
        }
    }

    pub fn vectors(&self) -> &[f32] {
        self.mapped_vectors
            .as_ref()
            .map_or(&self.vectors, MappedVectors::as_slice)
    }

    pub fn is_mapped(&self) -> bool {
        self.mapped_vectors.is_some()
    }

    pub fn mapped_vector_bytes(&self) -> usize {
        self.vectors()
            .len()
            .saturating_mul(std::mem::size_of::<f32>())
    }

    pub fn resident_index_bytes(&self) -> usize {
        self.index.heap_bytes()
    }

    pub fn chunk_count(&self) -> usize {
        self.vectors().len().checked_div(self.dim).unwrap_or(0)
    }

    pub fn search(&self, query: &[f32], limit: usize) -> Vec<(usize, f32)> {
        self.index
            .search_flat(self.vectors(), self.dim, query, limit)
    }
}

pub fn save_semantic_ivf(
    path: &Path,
    fingerprint: [u8; 32],
    dim: usize,
    vectors: &[f32],
    index: &SemanticAnnIndex,
) -> Result<()> {
    save_semantic_ivf_with_publication(path, fingerprint, dim, vectors, index).map(|_| ())
}

pub fn save_semantic_ivf_with_publication(
    path: &Path,
    fingerprint: [u8; 32],
    dim: usize,
    vectors: &[f32],
    index: &SemanticAnnIndex,
) -> Result<bool> {
    save_semantic_ivf_with_ids(path, fingerprint, dim, vectors, index, None)
}

/// v3 save: append the chunk-id column (the `SELECT id ... ORDER BY id`
/// order the vectors were flattened in) so the embed pass resolves IVF
/// positions from the mmap instead of a full-table SQL load. `None` writes
/// a file without the section (v2 shape, v3 version); a length mismatch
/// with the vector population is rejected before any file is created.
pub fn save_semantic_ivf_with_ids(
    path: &Path,
    fingerprint: [u8; 32],
    dim: usize,
    vectors: &[f32],
    index: &SemanticAnnIndex,
    ids: Option<&[i64]>,
) -> Result<bool> {
    if dim == 0 || vectors.is_empty() || !vectors.len().is_multiple_of(dim) {
        return Err(crate::StoreError::Other(
            "semantic IVF vectors must be nonempty and dimension-aligned".into(),
        ));
    }
    let chunk_count = vectors.len() / dim;
    if let Some(ids) = ids {
        if ids.len() != chunk_count {
            return Err(crate::StoreError::Other(
                "semantic IVF ids must cover the vector population exactly".into(),
            ));
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut index_bytes = Vec::new();
    index.write_to(&mut index_bytes, dim)?;
    let k = read_u32(&mut Cursor::new(index_bytes.as_slice()))? as usize;
    if k == 0 || k > 256 || k > chunk_count || !index.validate_partition(chunk_count) {
        return Err(crate::StoreError::Other(
            "semantic IVF index does not partition the supplied vectors".into(),
        ));
    }
    let vector_offset = align_up(
        HEADER_SIZE
            .checked_add(index_bytes.len())
            .ok_or_else(|| crate::StoreError::Other("semantic IVF index is too large".into()))?,
        VECTOR_ALIGNMENT,
    )
    .ok_or_else(|| crate::StoreError::Other("semantic IVF alignment overflow".into()))?;
    let vector_len = vectors.len() * std::mem::size_of::<f32>();
    // i64 alignment for the trailing ids section; only padding between
    // vectors and ids ever exists, never after ids.
    let ids_offset = align_up(
        vector_offset
            .checked_add(vector_len)
            .ok_or_else(|| crate::StoreError::Other("semantic IVF size overflow".into()))?,
        std::mem::align_of::<i64>(),
    )
    .ok_or_else(|| crate::StoreError::Other("semantic IVF alignment overflow".into()))?;
    reap_stale_temporaries(path);
    let temporary = temporary_path(path);
    let result = write_ivf_temporary(
        &temporary,
        path,
        IvfHeader {
            chunk_count,
            dim,
            fingerprint,
            k,
            index_len: index_bytes.len(),
            vector_offset,
        },
        &index_bytes,
        vectors,
        ids,
        ids_offset,
    );
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Write IVF payload to a create-new temporary, fsync, then atomically replace `path`.
#[allow(clippy::too_many_arguments)]
fn write_ivf_temporary(
    temporary: &Path,
    path: &Path,
    header: IvfHeader,
    index_bytes: &[u8],
    vectors: &[f32],
    ids: Option<&[i64]>,
    ids_offset: usize,
) -> Result<bool> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)?;
    write_header(&mut file, header)?;
    file.write_all(index_bytes)?;
    write_zeros(
        &mut file,
        header.vector_offset - HEADER_SIZE - index_bytes.len(),
    )?;
    file.write_all(bytemuck::cast_slice(vectors))?;
    if let Some(ids) = ids {
        let vector_end = header.vector_offset + vectors.len() * std::mem::size_of::<f32>();
        write_zeros(&mut file, ids_offset - vector_end)?;
        file.write_all(bytemuck::cast_slice(ids))?;
    }
    file.sync_all()?;
    Ok(replace_file(temporary, path)?)
}

#[derive(Debug, Clone)]
pub struct LazySemanticIvf {
    pub fingerprint: [u8; 32],
    pub dim: usize,
    chunk_count: usize,
    index: SemanticAnnIndex,
    mapped_vectors: Option<MappedVectors>,
    mapped_ids: Option<MappedIds>,
}

impl LazySemanticIvf {
    /// Chunk ids in flatten order, when the sidecar carries the v3 ids
    /// section. `None` on v2 sidecars — callers resolve ids from SQL.
    pub fn ids(&self) -> Option<&[i64]> {
        self.mapped_ids.as_ref().map(MappedIds::as_slice)
    }

    pub fn candidate_indices(&self, query: &[f32], probes: Option<usize>) -> Vec<usize> {
        self.index.candidate_indices(query, probes)
    }

    pub fn chunk_count(&self) -> usize {
        self.chunk_count
    }

    pub fn vectors(&self) -> Option<&[f32]> {
        self.mapped_vectors.as_ref().map(MappedVectors::as_slice)
    }

    /// Rank probed IVF members from the mmap payload. `None` if this sidecar
    /// has no mapped vectors (should not happen for a successful lazy load).
    pub fn search(
        &self,
        query: &[f32],
        limit: usize,
        probes: Option<usize>,
    ) -> Option<Vec<(usize, f32)>> {
        let _span = crate::perf_profile::Span::start(
            "semantic_ivf_search",
            "semantic",
            "LazySemanticIvf::search mmap score",
        );
        let flat = self.vectors()?;
        if self.dim == 0 || !flat.len().is_multiple_of(self.dim) {
            return None;
        }
        Some(
            self.index
                .search_flat_with_probes(flat, self.dim, query, limit, probes),
        )
    }

    /// Rank an explicit member set from the mmap payload (hybrid cascade files).
    pub fn search_members(
        &self,
        query: &[f32],
        members: &[usize],
        limit: usize,
    ) -> Option<Vec<(usize, f32)>> {
        let _span = crate::perf_profile::Span::start(
            "semantic_ivf_search_members",
            "semantic",
            "LazySemanticIvf::search_members mmap score",
        );
        let flat = self.vectors()?;
        if self.dim == 0 || !flat.len().is_multiple_of(self.dim) {
            return None;
        }
        Some(
            self.index
                .search_flat_members(flat, self.dim, query, members, limit),
        )
    }
}

struct LazyIvfMemo {
    path: PathBuf,
    fingerprint: [u8; 32],
    ivf: Arc<LazySemanticIvf>,
}

static LAZY_IVF_CACHE: OnceLock<Mutex<Option<LazyIvfMemo>>> = OnceLock::new();

fn lock_clear_on_poison<T>(mutex: &Mutex<T>, clear: impl FnOnce(&mut T)) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            mutex.clear_poison();
            let mut guard = PoisonError::into_inner(poisoned);
            clear(&mut guard);
            guard
        }
    }
}

fn lazy_ivf_cache() -> &'static Mutex<Option<LazyIvfMemo>> {
    LAZY_IVF_CACHE.get_or_init(|| Mutex::new(None))
}

pub fn load_semantic_ivf_index(
    path: &Path,
    expected_fingerprint: [u8; 32],
) -> Result<Option<Arc<LazySemanticIvf>>> {
    let _span = crate::perf_profile::Span::start(
        "semantic_ivf_load",
        "semantic",
        "load_semantic_ivf_index (cached mmap)",
    );
    {
        let guard = lock_clear_on_poison(lazy_ivf_cache(), |slot| *slot = None);
        if let Some(memo) = guard.as_ref() {
            if memo.path == path && memo.fingerprint == expected_fingerprint {
                return Ok(Some(Arc::clone(&memo.ivf)));
            }
        }
    }
    let Some(mapped) = map_and_parse(path, Some(expected_fingerprint))? else {
        return Ok(None);
    };
    let mapped_ids = mapped.ids_bytes.map(|bytes| MappedIds {
        mmap: Arc::clone(&mapped.mmap),
        bytes,
    });
    let mapped_vectors = MappedVectors {
        mmap: mapped.mmap,
        bytes: mapped.vector_bytes,
    };
    mapped_vectors.prefault();
    let ivf = Arc::new(LazySemanticIvf {
        fingerprint: mapped.header.fingerprint,
        dim: mapped.header.dim,
        chunk_count: mapped.header.chunk_count,
        index: mapped.index,
        mapped_vectors: Some(mapped_vectors),
        mapped_ids,
    });
    *lock_clear_on_poison(lazy_ivf_cache(), |slot| *slot = None) = Some(LazyIvfMemo {
        path: path.to_path_buf(),
        fingerprint: expected_fingerprint,
        ivf: Arc::clone(&ivf),
    });
    Ok(Some(ivf))
}

pub fn load_semantic_ivf(
    path: &Path,
    expected_fingerprint: [u8; 32],
) -> Result<Option<PersistedSemanticIvf>> {
    load_semantic_ivf_inner(path, Some(expected_fingerprint))
}

/// Read only the sidecar's stored fingerprint, without loading vectors (d3l5).
///
/// Used to report a generation-mismatched sidecar as a degraded channel instead
/// of silently falling back to brute force as if nothing were wrong.
pub fn peek_semantic_ivf_fingerprint(path: &Path) -> Option<[u8; 32]> {
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; HEADER_SIZE];
    file.read_exact(&mut header).ok()?;
    Some(read_header(&header, None)?.fingerprint)
}

pub fn load_semantic_ivf_unchecked(path: &Path) -> Result<Option<PersistedSemanticIvf>> {
    load_semantic_ivf_inner(path, None)
}

#[derive(Debug, Clone, Copy)]
struct IvfHeader {
    chunk_count: usize,
    dim: usize,
    fingerprint: [u8; 32],
    k: usize,
    index_len: usize,
    vector_offset: usize,
}

struct ParsedMapping {
    mmap: Arc<Mmap>,
    header: IvfHeader,
    index: SemanticAnnIndex,
    vector_bytes: Range<usize>,
    ids_bytes: Option<Range<usize>>,
}

fn load_semantic_ivf_inner(
    path: &Path,
    expected_fingerprint: Option<[u8; 32]>,
) -> Result<Option<PersistedSemanticIvf>> {
    let Some(mapped) = map_and_parse(path, expected_fingerprint)? else {
        return Ok(None);
    };
    let vectors = bytemuck::try_cast_slice::<u8, f32>(&mapped.mmap[mapped.vector_bytes.clone()])
        .expect("validated semantic IVF vector alignment")
        .to_vec();
    Ok(Some(PersistedSemanticIvf {
        fingerprint: mapped.header.fingerprint,
        dim: mapped.header.dim,
        vectors,
        mapped_vectors: Some(MappedVectors {
            mmap: mapped.mmap,
            bytes: mapped.vector_bytes,
        }),
        index: mapped.index,
    }))
}

fn map_and_parse(
    path: &Path,
    expected_fingerprint: Option<[u8; 32]>,
) -> Result<Option<ParsedMapping>> {
    let file = match open_mappable_file(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() < HEADER_SIZE as u64 {
        return Ok(None);
    }
    // SAFETY: this module never mutates a published sidecar in place. Writers create and
    // fsync a separate inode before replacement, so an existing read-only mapping remains
    // stable for its lifetime even while another process publishes a newer sidecar.
    let mmap = Arc::new(map_readonly(&file)?);
    let Some(header) = read_header(&mmap, expected_fingerprint) else {
        return Ok(None);
    };
    let Some(index_end) = HEADER_SIZE
        .checked_add(header.index_len)
        .filter(|&end| end <= header.vector_offset && header.vector_offset <= mmap.len())
    else {
        return Ok(None);
    };
    let Some(vector_len) = header
        .chunk_count
        .checked_mul(header.dim)
        .and_then(|count| count.checked_mul(std::mem::size_of::<f32>()))
    else {
        return Ok(None);
    };
    let Some(vector_end) = header
        .vector_offset
        .checked_add(vector_len)
        .filter(|&end| end <= mmap.len())
    else {
        return Ok(None);
    };
    // v3 trailing ids section: chunk_count i64s at i64 alignment past the
    // vectors, present iff the file ends exactly there. A file ending at
    // vector_end is the v2 shape (no ids). Anything else is truncated or
    // corrupt: reject rather than guess.
    let Some(ids_start) = align_up(vector_end, std::mem::align_of::<i64>()) else {
        return Ok(None);
    };
    let ids_end = header
        .chunk_count
        .checked_mul(std::mem::size_of::<i64>())
        .and_then(|len| ids_start.checked_add(len));
    let ids_bytes = match ids_end {
        Some(ids_end) if ids_end == mmap.len() => {
            let range = ids_start..ids_end;
            if bytemuck::try_cast_slice::<u8, i64>(&mmap[range.clone()]).is_err() {
                return Ok(None);
            }
            Some(range)
        }
        _ if vector_end == mmap.len() => None,
        _ => return Ok(None),
    };
    if header.vector_offset % VECTOR_ALIGNMENT != 0 {
        return Ok(None);
    }
    let index_bytes = &mmap[HEADER_SIZE..index_end];
    let index = match SemanticAnnIndex::read_clusters_bounded(
        index_bytes,
        header.k,
        header.dim,
        header.chunk_count,
    ) {
        Ok(index) => index,
        Err(_) => return Ok(None),
    };
    let vector_bytes = header.vector_offset..vector_end;
    if bytemuck::try_cast_slice::<u8, f32>(&mmap[vector_bytes.clone()]).is_err() {
        return Ok(None);
    }
    Ok(Some(ParsedMapping {
        mmap,
        header,
        index,
        vector_bytes,
        ids_bytes,
    }))
}

fn read_header(bytes: &[u8], expected_fingerprint: Option<[u8; 32]>) -> Option<IvfHeader> {
    let mut reader = Cursor::new(bytes.get(..HEADER_SIZE)?);
    let mut magic = [0u8; 6];
    reader.read_exact(&mut magic).ok()?;
    if &magic != MAGIC {
        return None;
    }
    let version = read_u32(&mut reader).ok()?;
    if !(MIN_READER_VERSION..=VERSION).contains(&version) {
        return None;
    }
    if read_u16(&mut reader).ok()? as usize != HEADER_SIZE {
        return None;
    }
    let chunk_count = usize::try_from(read_u64(&mut reader).ok()?).ok()?;
    let dim = read_u32(&mut reader).ok()? as usize;
    let mut fingerprint = [0u8; 32];
    reader.read_exact(&mut fingerprint).ok()?;
    if expected_fingerprint.is_some_and(|expected| fingerprint != expected) {
        return None;
    }
    let k = read_u32(&mut reader).ok()? as usize;
    let index_len = usize::try_from(read_u64(&mut reader).ok()?).ok()?;
    let vector_offset = usize::try_from(read_u64(&mut reader).ok()?).ok()?;
    let mut reserved = [0u8; 4];
    reader.read_exact(&mut reserved).ok()?;
    if reserved != [0; 4] || chunk_count == 0 || dim == 0 || k == 0 || k > 256 || k > chunk_count {
        return None;
    }
    Some(IvfHeader {
        chunk_count,
        dim,
        fingerprint,
        k,
        index_len,
        vector_offset,
    })
}

fn write_header(writer: &mut impl Write, header: IvfHeader) -> std::io::Result<()> {
    writer.write_all(MAGIC)?;
    writer.write_all(&VERSION.to_le_bytes())?;
    writer.write_all(&(HEADER_SIZE as u16).to_le_bytes())?;
    writer.write_all(&(header.chunk_count as u64).to_le_bytes())?;
    writer.write_all(&(header.dim as u32).to_le_bytes())?;
    writer.write_all(&header.fingerprint)?;
    writer.write_all(&(header.k as u32).to_le_bytes())?;
    writer.write_all(&(header.index_len as u64).to_le_bytes())?;
    writer.write_all(&(header.vector_offset as u64).to_le_bytes())?;
    writer.write_all(&[0; 4])?;
    Ok(())
}

fn read_u16(reader: &mut impl Read) -> std::io::Result<u16> {
    let mut bytes = [0u8; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(reader: &mut impl Read) -> std::io::Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> std::io::Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn write_zeros(writer: &mut impl Write, mut count: usize) -> std::io::Result<()> {
    const ZEROS: [u8; 4096] = [0; 4096];
    while count > 0 {
        let length = count.min(ZEROS.len());
        writer.write_all(&ZEROS[..length])?;
        count -= length;
    }
    Ok(())
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment.checked_sub(1)?)
        .map(|value| value / alignment * alignment)
}

fn reap_stale_temporaries(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(SEMANTIC_IVF_FILE);
    let prefix = format!(".{name}.");
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if !file_name.starts_with(&prefix) || !file_name.ends_with(".tmp") {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age.as_secs() >= 3_600);
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn temporary_path(path: &Path) -> std::path::PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(SEMANTIC_IVF_FILE);
    path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), sequence))
}

#[cfg(windows)]
fn open_mappable_file(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ_WRITE_DELETE)
        .open(path)
}

#[cfg(not(windows))]
fn open_mappable_file(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(unix)]
fn sync_parent(destination: &Path) -> std::io::Result<()> {
    if let Some(parent) = destination.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<bool> {
    match fs::rename(source, destination) {
        Ok(()) => {
            sync_parent(destination)?;
            Ok(true)
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || matches!(error.raw_os_error(), Some(32) | Some(33)) =>
        {
            // Windows cannot replace a file while another process retains a section
            // mapping. Keep the prior valid sidecar and use the newly built in-memory
            // index for this process; a later rebuild can publish after readers exit.
            fs::remove_file(source)?;
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<bool> {
    fs::rename(source, destination)?;
    sync_parent(destination)?;
    Ok(true)
}
