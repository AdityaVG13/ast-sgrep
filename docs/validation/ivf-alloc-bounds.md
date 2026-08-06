# IVF header allocation bounds (`l115`)

`semantic_ivf` rejects headers when `dim == 0`, `chunk_count == 0`, or cluster
count `k` is outside `1..=256` and `k <= chunk_count` before allocating vector
views. Mapped readers validate vector byte ranges against `mmap.len()` before
`bytemuck` casts. See `crates/ast-sgrep-core/src/semantic_ivf.rs` and
`docs/validation/semantic-ivf-mmap.md`.
