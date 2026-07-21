use crate::semantic_ann::SemanticAnnIndex;
use crate::Result;
use blake3::Hasher;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
const MAGIC: &[u8; 6] = b"ASIVF\0";
const VERSION: u32 = 1;
pub const SEMANTIC_IVF_FILE: &str = "semantic.ivf";
pub fn semantic_ivf_path(index_db: &Path) -> std::path::PathBuf {
    index_db
        .parent()
        .map(|p| p.join(SEMANTIC_IVF_FILE))
        .unwrap_or_else(|| Path::new(SEMANTIC_IVF_FILE).to_path_buf())
}
pub fn invalidate_semantic_ivf(index_db: &Path) -> Result<()> {
    let path = semantic_ivf_path(index_db);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
pub fn compute_ann_fingerprint(
    chunk_count: usize,
    max_chunk_id: i64,
    dim: usize,
    embed_backend: Option<&str>,
) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(b"asgrep-semantic-ivf-v1");
    h.update(&(chunk_count as u64).to_le_bytes());
    h.update(&max_chunk_id.to_le_bytes());
    h.update(&(dim as u32).to_le_bytes());
    h.update(embed_backend.unwrap_or("semantic").as_bytes());
    *h.finalize().as_bytes()
}
#[derive(Debug, Clone)]
pub struct PersistedSemanticIvf {
    pub fingerprint: [u8; 32],
    pub dim: usize,
    pub vectors: Vec<f32>,
    pub index: SemanticAnnIndex,
}
impl PersistedSemanticIvf {
    pub fn chunk_count(&self) -> usize {
        self.vectors.len().checked_div(self.dim).unwrap_or(0)
    }
    pub fn search(&self, query: &[f32], limit: usize) -> Vec<(usize, f32)> {
        self.index
            .search_flat(&self.vectors, self.dim, query, limit)
    }
}
pub fn save_semantic_ivf(
    path: &Path,
    fingerprint: [u8; 32],
    dim: usize,
    vectors: &[f32],
    index: &SemanticAnnIndex,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let chunk_count = vectors.len().checked_div(dim).unwrap_or(0);
    let mut file = File::create(path)?;
    file.write_all(MAGIC)?;
    file.write_all(&VERSION.to_le_bytes())?;
    file.write_all(&(chunk_count as u64).to_le_bytes())?;
    file.write_all(&(dim as u32).to_le_bytes())?;
    file.write_all(&fingerprint)?;
    index.write_to(&mut file, dim)?;
    for &v in vectors {
        file.write_all(&v.to_le_bytes())?;
    }
    Ok(())
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
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let Some(header) = read_ivf_header(&mut file, Some(expected_fingerprint))? else {
        return Ok(None);
    };
    let index = SemanticAnnIndex::read_clusters_from(&mut file, header.k, header.dim)?;
    if !index.validate_member_indices(header.chunk_count) {
        return Ok(None);
    }
    Ok(Some(LazySemanticIvf {
        fingerprint: header.fingerprint,
        dim: header.dim,
        chunk_count: header.chunk_count,
        index,
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
struct IvfHeader {
    chunk_count: usize,
    dim: usize,
    fingerprint: [u8; 32],
    k: usize,
}
fn read_u32<R: Read>(r: &mut R) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_u64<R: Read>(r: &mut R) -> std::io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
fn read_ivf_header<R: Read>(
    reader: &mut R,
    expected_fingerprint: Option<[u8; 32]>,
) -> std::io::Result<Option<IvfHeader>> {
    let mut magic = [0u8; 6];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC || read_u32(reader)? != VERSION {
        return Ok(None);
    }
    let chunk_count = read_u64(reader)? as usize;
    let dim = read_u32(reader)? as usize;
    let mut fingerprint = [0u8; 32];
    reader.read_exact(&mut fingerprint)?;
    if expected_fingerprint.is_some_and(|want| fingerprint != want) {
        return Ok(None);
    }
    Ok(Some(IvfHeader {
        chunk_count,
        dim,
        fingerprint,
        k: read_u32(reader)? as usize,
    }))
}
fn try_parse_semantic_ivf_from_reader<R: Read>(
    reader: &mut R,
    expected_fingerprint: Option<[u8; 32]>,
) -> std::io::Result<Option<PersistedSemanticIvf>> {
    let Some(header) = read_ivf_header(reader, expected_fingerprint)? else {
        return Ok(None);
    };
    let index = SemanticAnnIndex::read_clusters_from(reader, header.k, header.dim)?;
    let vector_bytes = match header
        .chunk_count
        .checked_mul(header.dim)
        .and_then(|n| n.checked_mul(4))
    {
        Some(n) => n,
        None => return Ok(None),
    };
    let mut raw = vec![0u8; vector_bytes];
    reader.read_exact(&mut raw)?;
    let vectors: Vec<f32> = raw
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if vectors.len() != header.chunk_count * header.dim
        || !index.validate_member_indices(header.chunk_count)
    {
        return Ok(None);
    }
    Ok(Some(PersistedSemanticIvf {
        fingerprint: header.fingerprint,
        dim: header.dim,
        vectors,
        index,
    }))
}
fn load_semantic_ivf_inner(
    path: &Path,
    expected_fingerprint: Option<[u8; 32]>,
) -> Result<Option<PersistedSemanticIvf>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    Ok(
        try_parse_semantic_ivf_from_reader(&mut file, expected_fingerprint)
            .ok()
            .flatten(),
    )
}
