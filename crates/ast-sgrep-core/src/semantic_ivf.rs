//! On-disk IVF sidecar for semantic vectors (`.asgrep/semantic.ivf`).

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use blake3::Hasher;

use crate::Result;
use crate::semantic_ann::SemanticAnnIndex;

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
    let mut hasher = Hasher::new();
    hasher.update(b"asgrep-semantic-ivf-v1");
    hasher.update(&(chunk_count as u64).to_le_bytes());
    hasher.update(&max_chunk_id.to_le_bytes());
    hasher.update(&(dim as u32).to_le_bytes());
    hasher.update(embed_backend.unwrap_or("semantic").as_bytes());
    *hasher.finalize().as_bytes()
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
        if self.dim == 0 {
            return 0;
        }
        self.vectors.len() / self.dim
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
    let chunk_count = if dim == 0 { 0 } else { vectors.len() / dim };
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

pub fn load_semantic_ivf(path: &Path, expected_fingerprint: [u8; 32]) -> Result<Option<PersistedSemanticIvf>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let loaded = (|| -> std::io::Result<Option<PersistedSemanticIvf>> {
        let mut magic = [0u8; 6];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Ok(None);
        }
        let mut ver_buf = [0u8; 4];
        file.read_exact(&mut ver_buf)?;
        if u32::from_le_bytes(ver_buf) != VERSION {
            return Ok(None);
        }
        let mut count_buf = [0u8; 8];
        file.read_exact(&mut count_buf)?;
        let chunk_count = u64::from_le_bytes(count_buf) as usize;
        let mut dim_buf = [0u8; 4];
        file.read_exact(&mut dim_buf)?;
        let dim = u32::from_le_bytes(dim_buf) as usize;
        let mut fingerprint = [0u8; 32];
        file.read_exact(&mut fingerprint)?;
        if fingerprint != expected_fingerprint {
            return Ok(None);
        }
        let mut k_buf = [0u8; 4];
        file.read_exact(&mut k_buf)?;
        let k = u32::from_le_bytes(k_buf) as usize;
        let index = SemanticAnnIndex::read_clusters_from(&mut file, k, dim)?;
        let vector_bytes = chunk_count
            .checked_mul(dim)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "ivf vector size overflow"))?;
        let mut raw = vec![0u8; vector_bytes];
        file.read_exact(&mut raw)?;
        let vectors: Vec<f32> = raw
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if vectors.len() != chunk_count * dim {
            return Ok(None);
        }
        if !index.validate_member_indices(chunk_count) {
            return Ok(None);
        }
        Ok(Some(PersistedSemanticIvf {
            fingerprint,
            dim,
            vectors,
            index,
        }))
    })();
    match loaded {
        Ok(v) => Ok(v),
        Err(_) => Ok(None),
    }
}
