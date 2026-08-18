//! Sealed read-only mmap boundary for ast-sgrep.
//!
//! **Policy (`ast-sgrep-p7l3`):** first-party product crates use
//! `#![forbid(unsafe_code)]`. The only hand-written `unsafe` in this workspace
//! lives here, wrapping `memmap2::MmapOptions::map` behind a safe API. The
//! separate N-API crate permits macro-generated FFI glue only.
//!
//! Callers must not mutate a published sidecar inode in place; writers fsync
//! and rename a separate file so existing mappings keep a stable view.

#![doc(html_no_source)]

use memmap2::MmapOptions;
use std::fs::File;
use std::io;

/// Map an open file read-only.
///
/// # Safety boundary
///
/// `memmap2` requires `unsafe` because the OS may change mapped bytes if
/// another process truncates or writes the same inode. Product code never
/// mutates a published IVF sidecar in place (write → fsync → rename), so this
/// wrapper is sound for that publishing protocol.
pub fn map_readonly(file: &File) -> io::Result<Mmap> {
    // SAFETY: callers map a shared read-only handle under the rename-publish
    // protocol documented above; this crate is the sealed unsafe boundary.
    unsafe { MmapOptions::new().map(file) }
}

pub use memmap2::Mmap;

