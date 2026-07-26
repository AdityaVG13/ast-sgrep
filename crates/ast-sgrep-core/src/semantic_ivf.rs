use crate::semantic_ann::SemanticAnnIndex;
use crate::Result;
use blake3::Hasher;
use memmap2::{Mmap, MmapOptions};
use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const MAGIC: &[u8; 6] = b"ASIVF\0";
const VERSION: u32 = 2;
const HEADER_SIZE: usize = 80;
const VECTOR_ALIGNMENT: usize = 4096;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub const SEMANTIC_IVF_FILE: &str = "semantic.ivf";

pub fn semantic_ivf_path(index_db: &Path) -> std::path::PathBuf {
    index_db
        .parent()
        .map(|parent| parent.join(SEMANTIC_IVF_FILE))
        .unwrap_or_else(|| Path::new(SEMANTIC_IVF_FILE).to_path_buf())
}

pub fn invalidate_semantic_ivf(index_db: &Path) -> Result<()> {
    let path = semantic_ivf_path(index_db);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if benign_invalidation_error(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
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
) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(b"asgrep-semantic-ivf-v2");
    hasher.update(&(chunk_count as u64).to_le_bytes());
    hasher.update(&max_chunk_id.to_le_bytes());
    hasher.update(&(dim as u32).to_le_bytes());
    hasher.update(embed_backend.unwrap_or("semantic").as_bytes());
    *hasher.finalize().as_bytes()
}

#[derive(Debug, Clone)]
enum VectorStorage {
    Owned(Vec<f32>),
    Mapped {
        mmap: Arc<Mmap>,
        bytes: Range<usize>,
    },
}

impl VectorStorage {
    fn as_slice(&self) -> &[f32] {
        match self {
            Self::Owned(vectors) => vectors,
            Self::Mapped { mmap, bytes } => bytemuck::try_cast_slice(&mmap[bytes.clone()])
                .expect("validated semantic IVF vector alignment"),
        }
    }

    fn is_mapped(&self) -> bool {
        matches!(self, Self::Mapped { .. })
    }
}

#[derive(Debug, Clone)]
pub struct PersistedSemanticIvf {
    pub fingerprint: [u8; 32],
    pub dim: usize,
    vectors: VectorStorage,
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
            vectors: VectorStorage::Owned(vectors),
            index,
        }
    }

    pub fn vectors(&self) -> &[f32] {
        self.vectors.as_slice()
    }

    pub fn is_mapped(&self) -> bool {
        self.vectors.is_mapped()
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
) -> Result<bool> {
    if dim == 0 || vectors.is_empty() || !vectors.len().is_multiple_of(dim) {
        return Err(crate::StoreError::Other(
            "semantic IVF vectors must be nonempty and dimension-aligned".into(),
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let chunk_count = vectors.len() / dim;
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
    reap_stale_temporaries(path);
    let temporary = temporary_path(path);
    let result = (|| -> Result<bool> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        write_header(
            &mut file,
            IvfHeader {
                chunk_count,
                dim,
                fingerprint,
                k,
                index_len: index_bytes.len(),
                vector_offset,
            },
        )?;
        file.write_all(&index_bytes)?;
        write_zeros(&mut file, vector_offset - HEADER_SIZE - index_bytes.len())?;
        file.write_all(bytemuck::cast_slice(vectors))?;
        file.sync_all()?;
        Ok(replace_file(&temporary, path)?)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[derive(Debug, Clone)]
pub struct LazySemanticIvf {
    pub fingerprint: [u8; 32],
    pub dim: usize,
    chunk_count: usize,
    index: SemanticAnnIndex,
}

impl LazySemanticIvf {
    pub fn candidate_indices(&self, query: &[f32], probes: Option<usize>) -> Vec<usize> {
        self.index.candidate_indices(query, probes)
    }

    pub fn chunk_count(&self) -> usize {
        self.chunk_count
    }
}

pub fn load_semantic_ivf_index(
    path: &Path,
    expected_fingerprint: [u8; 32],
) -> Result<Option<LazySemanticIvf>> {
    let Some(mapped) = map_and_parse(path, Some(expected_fingerprint))? else {
        return Ok(None);
    };
    Ok(Some(LazySemanticIvf {
        fingerprint: mapped.header.fingerprint,
        dim: mapped.header.dim,
        chunk_count: mapped.header.chunk_count,
        index: mapped.index,
    }))
}

pub fn load_semantic_ivf(
    path: &Path,
    expected_fingerprint: [u8; 32],
) -> Result<Option<PersistedSemanticIvf>> {
    load_semantic_ivf_inner(path, Some(expected_fingerprint))
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
}

fn load_semantic_ivf_inner(
    path: &Path,
    expected_fingerprint: Option<[u8; 32]>,
) -> Result<Option<PersistedSemanticIvf>> {
    let Some(mapped) = map_and_parse(path, expected_fingerprint)? else {
        return Ok(None);
    };
    Ok(Some(PersistedSemanticIvf {
        fingerprint: mapped.header.fingerprint,
        dim: mapped.header.dim,
        vectors: VectorStorage::Mapped {
            mmap: mapped.mmap,
            bytes: mapped.vector_bytes,
        },
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
    if !metadata.is_file() {
        return Ok(None);
    }
    if metadata.len() < HEADER_SIZE as u64 {
        return Ok(None);
    }
    // SAFETY: this module never mutates a published sidecar in place. Writers create and
    // fsync a separate inode before replacement, so an existing read-only mapping remains
    // stable for its lifetime even while another process publishes a newer sidecar.
    let mmap = Arc::new(map_readonly(&file)?);
    let Some(header) = read_header(&mmap, expected_fingerprint) else {
        return Ok(None);
    };
    let index_end = match HEADER_SIZE.checked_add(header.index_len) {
        Some(end) if end <= header.vector_offset && header.vector_offset <= mmap.len() => end,
        _ => return Ok(None),
    };
    let vector_len = match header
        .chunk_count
        .checked_mul(header.dim)
        .and_then(|count| count.checked_mul(std::mem::size_of::<f32>()))
    {
        Some(length) => length,
        None => return Ok(None),
    };
    let vector_end = match header.vector_offset.checked_add(vector_len) {
        Some(end) if end == mmap.len() => end,
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
    }))
}

fn read_header(bytes: &[u8], expected_fingerprint: Option<[u8; 32]>) -> Option<IvfHeader> {
    let mut reader = Cursor::new(bytes.get(..HEADER_SIZE)?);
    let mut magic = [0u8; 6];
    reader.read_exact(&mut magic).ok()?;
    if &magic != MAGIC || read_u32(&mut reader).ok()? != VERSION {
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

#[allow(unsafe_code)]
fn map_readonly(file: &File) -> std::io::Result<Mmap> {
    // SAFETY: callers map a shared read-only handle. This module never mutates a
    // published inode in place; writers fsync and rename a separate file.
    unsafe { MmapOptions::new().map(file) }
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
