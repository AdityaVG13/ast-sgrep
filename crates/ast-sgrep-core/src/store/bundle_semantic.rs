//! Semantic concat tables packed into the read bundle.
//!
//! Chunk metadata, interned `sc.text`, and f16 primary vectors live in the
//! mmap. Ranking converts f16 on the stack per row; decode never materializes
//! a corpus-sized f32. SQL remains the `ASGREP_READPATH=sql` control arm.
//!
//! `SEC_SEM_VEC` may trail the concat payload with per-field f16 planes
//! (`count` mask bytes, then 5 × `count*dim` f16). Older concat-only files
//! keep the prefix; readers without a trailer fall back to SQL field blobs.

use super::bundle_graph::{file_idx, file_index, Intern};
use super::read_bundle::ReadBundle;
use super::IndexStore;
use crate::semantic_chunk::{FieldVectorMask, SemanticFieldVectors};
use crate::Result;
use ast_sgrep_embed::SemanticChunkRow;
use std::sync::Arc;

const FIELD_PLANES: usize = 5;

pub(crate) const SEC_SEM_META: usize = 22;
pub(crate) const SEC_SEM_VEC: usize = 23;
/// id i64 + file_idx u32 + start u32 + end u32 + name_idx u32 + text_idx u32.
pub(crate) const SEM_ROW: usize = 28;
/// Pre-rev-5 rows omitted interned `sc.text`.
const SEM_ROW_LEGACY: usize = 24;

#[derive(Default)]
pub(crate) struct SemanticSections {
    pub meta: Vec<u8>,
    pub vecs: Vec<u8>,
}

pub struct SemanticView {
    pub ids: Arc<Vec<i64>>,
    pub(crate) file_idx: Arc<Vec<u32>>,
    pub(crate) start: Arc<Vec<u32>>,
    pub(crate) end: Arc<Vec<u32>>,
    pub(crate) name_idx: Arc<Vec<u32>>,
    pub(crate) text_idx: Arc<Vec<u32>>,
    pub(crate) intern_offs: Arc<Vec<u32>>,
    pub(crate) intern_names_at: usize,
    pub(crate) intern_names_len: usize,
    pub(crate) path_order: Arc<Vec<u32>>,
    pub(crate) id_order: Arc<Vec<u32>>,
    pub count: usize,
    pub dim: usize,
}

impl SemanticView {
    pub fn f16_bytes<'a>(&self, bundle: &'a ReadBundle) -> Option<&'a [u8]> {
        let vecs = bundle.section(SEC_SEM_VEC);
        let need = 8 + self.count.saturating_mul(self.dim).saturating_mul(2);
        if vecs.len() < need {
            return None;
        }
        Some(&vecs[8..need])
    }

    pub fn row(&self, bundle: &ReadBundle, i: usize) -> Option<SemanticChunkRow> {
        if i >= self.count {
            return None;
        }
        let meta = bundle.section(SEC_SEM_META);
        let names_end = self.intern_names_at.saturating_add(self.intern_names_len);
        if meta.len() < names_end {
            return None;
        }
        let names = &meta[self.intern_names_at..names_end];
        let offs = self.intern_offs.as_slice();
        let name = intern_get(names, offs, self.name_idx[i] as usize)?.to_owned();
        let text = if self.text_idx[i] == u32::MAX {
            String::new()
        } else {
            intern_get(names, offs, self.text_idx[i] as usize)?.to_owned()
        };
        let path = bundle.path(self.file_idx[i] as usize).to_owned();
        Some((
            path,
            self.start[i],
            self.end[i],
            name,
            text,
            Vec::new(),
        ))
    }

    /// Packed row for a semantic chunk id. `id_order` is sorted by id, so this
    /// is a binary search even when pack order is not id order.
    pub fn index_of_id(&self, id: i64) -> Option<usize> {
        let order = self.id_order.as_slice();
        let ids = self.ids.as_slice();
        let slot = order
            .binary_search_by(|&i| ids[i as usize].cmp(&id))
            .ok()?;
        Some(order[slot] as usize)
    }

    pub fn row_by_id(&self, bundle: &ReadBundle, id: i64) -> Option<SemanticChunkRow> {
        self.row(bundle, self.index_of_id(id)?)
    }

    pub fn fields_by_id(
        &self,
        bundle: &ReadBundle,
        id: i64,
        mask: FieldVectorMask,
    ) -> Option<SemanticFieldVectors> {
        self.fields_at(bundle, self.index_of_id(id)?, mask)
    }

    pub fn fields_at(
        &self,
        bundle: &ReadBundle,
        i: usize,
        mask: FieldVectorMask,
    ) -> Option<SemanticFieldVectors> {
        if i >= self.count {
            return None;
        }
        let (masks, planes) = field_trailer(bundle, self)?;
        let bits = masks[i];
        let plane_bytes = self.count.saturating_mul(self.dim).saturating_mul(2);
        let row_off = i.saturating_mul(self.dim).saturating_mul(2);
        let take = |bit: u8, plane: usize, want: bool| {
            if !want || bits & bit == 0 {
                return None;
            }
            let start = plane.saturating_mul(plane_bytes).saturating_add(row_off);
            let end = start.saturating_add(self.dim.saturating_mul(2));
            let row = planes.get(start..end)?;
            if !row.len().is_multiple_of(2) {
                return None;
            }
            // Packed f16 stays packed. score_fields expands once into f32.
            Some(row.to_vec())
        };
        Some(SemanticFieldVectors {
            name: take(1, 0, mask.name),
            docs: take(2, 1, mask.docs),
            body: take(4, 2, mask.body),
            graph: take(8, 3, mask.graph),
            tests_examples: take(16, 4, mask.tests_examples),
        })
    }
}

pub(crate) fn pack_from_store(
    store: &IndexStore,
    rows: &[crate::store::IndexedLineRow],
) -> Result<SemanticSections> {
    let chunks = store.all_semantic_chunks(None)?;
    if chunks.is_empty() {
        return Ok(SemanticSections::default());
    }
    let ids = store.semantic_chunk_ids(None)?;
    if ids.len() != chunks.len() {
        return Err(crate::StoreError::Other(
            "semantic chunk id/row length mismatch while packing the read bundle".into(),
        )
        .into());
    }
    let dim = chunks[0].5.len();
    if dim == 0 || chunks.iter().any(|chunk| chunk.5.len() != dim) {
        return Err(crate::StoreError::Other(
            "semantic concat dim missing or mixed while packing the read bundle".into(),
        )
        .into());
    }
    let files = file_index(rows);
    let mut names = Intern::new();
    let mut meta_rows = Vec::with_capacity(chunks.len() * SEM_ROW);
    for (id, chunk) in ids.iter().zip(chunks.iter()) {
        let path = chunk.0.as_str();
        let Some(file_idx) = file_idx(&files, path) else {
            return Err(crate::StoreError::Other(format!(
                "semantic chunk path {path} missing from line bundle"
            ))
            .into());
        };
        let name_idx = names.intern(&chunk.3)?;
        let text_idx = names.intern(&chunk.4)?;
        meta_rows.extend_from_slice(&id.to_le_bytes());
        meta_rows.extend_from_slice(&file_idx.to_le_bytes());
        meta_rows.extend_from_slice(&chunk.1.to_le_bytes());
        meta_rows.extend_from_slice(&chunk.2.to_le_bytes());
        meta_rows.extend_from_slice(&name_idx.to_le_bytes());
        meta_rows.extend_from_slice(&text_idx.to_le_bytes());
    }
    let (mut name_bytes, name_offs) = names.finish();
    while name_bytes.len() % 4 != 0 {
        name_bytes.push(0);
    }
    let name_count = (name_offs.len() - 1) as u32;
    let mut meta = Vec::with_capacity(12 + name_bytes.len() + name_offs.len() * 4 + meta_rows.len());
    meta.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
    meta.extend_from_slice(&name_count.to_le_bytes());
    meta.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    meta.extend_from_slice(&name_bytes);
    meta.extend_from_slice(bytemuck::cast_slice(&name_offs));
    meta.extend_from_slice(&meta_rows);

    let n = ids.len();
    let mut vecs = Vec::with_capacity(8 + n * dim * 2 + n + FIELD_PLANES * n * dim * 2);
    vecs.extend_from_slice(&(dim as u32).to_le_bytes());
    vecs.extend_from_slice(&(n as u32).to_le_bytes());
    for chunk in &chunks {
        for lane in &chunk.5 {
            vecs.extend_from_slice(&f32_to_f16(*lane).to_le_bytes());
        }
    }
    append_field_trailer(&mut vecs, store, &ids, dim)?;
    Ok(SemanticSections { meta, vecs })
}

fn append_field_trailer(
    vecs: &mut Vec<u8>,
    store: &IndexStore,
    ids: &[i64],
    dim: usize,
) -> Result<()> {
    let Ok(fields) = store.semantic_chunk_field_vectors() else {
        return Ok(());
    };
    if fields.len() != ids.len()
        || fields
            .iter()
            .zip(ids.iter())
            .any(|((id, _), want)| id != want)
    {
        return Ok(());
    }
    let n = ids.len();
    let mut masks = vec![0u8; n];
    let mut planes = vec![Vec::with_capacity(n * dim * 2); FIELD_PLANES];
    for (i, (_, fv)) in fields.iter().enumerate() {
        let blobs = [
            fv.name.as_deref(),
            fv.docs.as_deref(),
            fv.body.as_deref(),
            fv.graph.as_deref(),
            fv.tests_examples.as_deref(),
        ];
        for (plane, blob) in blobs.into_iter().enumerate() {
            if push_f16_row(&mut planes[plane], blob, dim) {
                masks[i] |= 1 << plane;
            }
        }
    }
    vecs.extend_from_slice(&masks);
    for plane in &planes {
        vecs.extend_from_slice(plane);
    }
    Ok(())
}

fn push_f16_row(out: &mut Vec<u8>, blob: Option<&[u8]>, dim: usize) -> bool {
    let zeros = dim.saturating_mul(2);
    let Some(bytes) = blob.filter(|b| !b.is_empty()) else {
        out.extend(std::iter::repeat_n(0u8, zeros));
        return false;
    };
    let Ok(vec) = ast_sgrep_embed::embed_from_bytes(bytes) else {
        out.extend(std::iter::repeat_n(0u8, zeros));
        return false;
    };
    if vec.len() != dim {
        out.extend(std::iter::repeat_n(0u8, zeros));
        return false;
    }
    for lane in vec {
        out.extend_from_slice(&f32_to_f16(lane).to_le_bytes());
    }
    true
}

fn concat_bytes(count: usize, dim: usize) -> usize {
    8 + count.saturating_mul(dim).saturating_mul(2)
}

fn field_trailer_bytes(count: usize, dim: usize) -> usize {
    count.saturating_add(FIELD_PLANES.saturating_mul(count.saturating_mul(dim).saturating_mul(2)))
}

fn field_trailer<'a>(bundle: &'a ReadBundle, view: &SemanticView) -> Option<(&'a [u8], &'a [u8])> {
    let vecs = bundle.section(SEC_SEM_VEC);
    let concat = concat_bytes(view.count, view.dim);
    let need = field_trailer_bytes(view.count, view.dim);
    if vecs.len() < concat.saturating_add(need) {
        return None;
    }
    Some((
        &vecs[concat..concat + view.count],
        &vecs[concat + view.count..concat + need],
    ))
}

pub fn decode_view(bundle: &ReadBundle) -> Option<SemanticView> {
    let meta = bundle.section(SEC_SEM_META);
    let vecs = bundle.section(SEC_SEM_VEC);
    if meta.len() < 12 || vecs.len() < 8 {
        return None;
    }
    let name_bytes_len = u32::from_le_bytes(meta[0..4].try_into().ok()?) as usize;
    let name_count = u32::from_le_bytes(meta[4..8].try_into().ok()?) as usize;
    let count = u32::from_le_bytes(meta[8..12].try_into().ok()?) as usize;
    let names_at = 12;
    let offs_at = names_at + name_bytes_len;
    let offs_len = name_count.saturating_add(1).saturating_mul(4);
    let rows_at = offs_at + offs_len;
    let remain = meta.len().saturating_sub(rows_at);
    let stride = if remain == count.saturating_mul(SEM_ROW) {
        SEM_ROW
    } else if remain == count.saturating_mul(SEM_ROW_LEGACY) {
        SEM_ROW_LEGACY
    } else {
        return None;
    };
    let offs: &[u32] = bytemuck::try_cast_slice(&meta[offs_at..rows_at]).ok()?;
    if offs.len() != name_count + 1 {
        return None;
    }
    let dim = u32::from_le_bytes(vecs[0..4].try_into().ok()?) as usize;
    let vec_count = u32::from_le_bytes(vecs[4..8].try_into().ok()?) as usize;
    if dim == 0 || vec_count != count {
        return None;
    }
    let need = 8 + count.saturating_mul(dim).saturating_mul(2);
    if vecs.len() < need {
        return None;
    }
    let intern_offs = offs.to_vec();
    let mut ids = Vec::with_capacity(count);
    let mut file_idx = Vec::with_capacity(count);
    let mut start = Vec::with_capacity(count);
    let mut end = Vec::with_capacity(count);
    let mut name_idx = Vec::with_capacity(count);
    let mut text_idx = Vec::with_capacity(count);
    for i in 0..count {
        let row = &meta[rows_at + i * stride..rows_at + (i + 1) * stride];
        let id = i64::from_le_bytes(row[0..8].try_into().ok()?);
        let file = u32::from_le_bytes(row[8..12].try_into().ok()?);
        let line_start = u32::from_le_bytes(row[12..16].try_into().ok()?);
        let line_end = u32::from_le_bytes(row[16..20].try_into().ok()?);
        let name = u32::from_le_bytes(row[20..24].try_into().ok()?);
        let text = if stride == SEM_ROW {
            u32::from_le_bytes(row[24..28].try_into().ok()?)
        } else {
            u32::MAX
        };
        if (file as usize) >= bundle.file_count() || (name as usize) + 1 >= intern_offs.len() {
            return None;
        }
        ids.push(id);
        file_idx.push(file);
        start.push(line_start);
        end.push(line_end);
        name_idx.push(name);
        text_idx.push(text);
    }
    let mut path_order: Vec<u32> = (0..count as u32).collect();
    path_order.sort_by(|&a, &b| {
        let left = file_idx[a as usize] as usize;
        let right = file_idx[b as usize] as usize;
        bundle
            .path(left)
            .cmp(bundle.path(right))
            .then(a.cmp(&b))
    });
    let mut id_order: Vec<u32> = (0..count as u32).collect();
    id_order.sort_by(|&a, &b| ids[a as usize].cmp(&ids[b as usize]).then(a.cmp(&b)));
    Some(SemanticView {
        ids: Arc::new(ids),
        file_idx: Arc::new(file_idx),
        start: Arc::new(start),
        end: Arc::new(end),
        name_idx: Arc::new(name_idx),
        text_idx: Arc::new(text_idx),
        intern_offs: Arc::new(intern_offs),
        intern_names_at: names_at,
        intern_names_len: name_bytes_len,
        path_order: Arc::new(path_order),
        id_order: Arc::new(id_order),
        count,
        dim,
    })
}

fn intern_get<'a>(names: &'a [u8], offs: &[u32], idx: usize) -> Option<&'a str> {
    if idx + 1 >= offs.len() {
        return None;
    }
    let ns = offs[idx] as usize;
    let ne = offs[idx + 1] as usize;
    if ne < ns || ne > names.len() {
        return None;
    }
    std::str::from_utf8(&names[ns..ne]).ok()
}

pub fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x7f_ffff;
    if exp == 255 {
        return if mant == 0 {
            sign | 0x7c00
        } else {
            sign | 0x7e00
        };
    }
    let mut exp16 = exp - 127 + 15;
    if exp16 >= 31 {
        return sign | 0x7c00;
    }
    if exp16 <= 0 {
        if exp16 < -10 {
            return sign;
        }
        let mant = (mant | 0x80_0000) >> (1 - exp16);
        return sign | ((mant + 0x1000) >> 13) as u16;
    }
    let mut mant = mant + 0x1000;
    if mant >= 0x80_0000 {
        mant = 0;
        exp16 += 1;
        if exp16 >= 31 {
            return sign | 0x7c00;
        }
    }
    sign | ((exp16 as u16) << 10) | ((mant >> 13) as u16)
}

pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = (bits as u32) << 16 & 0x8000_0000;
    let exp = (bits >> 10) & 0x1f;
    let mant = bits & 0x3ff;
    let out = if exp == 0 {
        if mant == 0 {
            sign
        } else {
            let mut e = 127 - 15 + 1;
            let mut m = mant as u32;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x3ff) << 13)
        }
    } else if exp == 31 {
        sign | 0x7f80_0000 | ((mant as u32) << 13)
    } else {
        sign | (((exp as u32) + 127 - 15) << 23) | ((mant as u32) << 13)
    };
    f32::from_bits(out)
}
