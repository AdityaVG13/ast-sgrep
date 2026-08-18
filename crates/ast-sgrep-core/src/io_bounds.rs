//! Bounded file reads for indexing (dx4g / zbpc).

use crate::{Result, StoreError};
use std::fs::{File, Metadata};
use std::io::{BufRead, Read};
use std::path::{Component, Path};

/// Stable project-root handle used to open indexed files without following a
/// symlink swapped into any path component after watcher validation.
pub(crate) struct RootDir {
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
    #[cfg(windows)]
    dir: cap_std::fs::Dir,
    #[cfg(not(any(unix, windows)))]
    path: std::path::PathBuf,
}

pub(crate) struct CappedText {
    pub text: String,
    pub metadata: Metadata,
}

impl RootDir {
    pub fn open(root: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            use rustix::fs::{open, Mode, OFlags};
            let fd = open(
                root,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(std::io::Error::from)?;
            Ok(Self { fd })
        }
        #[cfg(windows)]
        {
            let dir = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
            Ok(Self { dir })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(Self {
                path: root.canonicalize()?,
            })
        }
    }

    pub fn read_text_capped(&self, rel: &Path, max_bytes: u64) -> Result<CappedText> {
        #[cfg(unix)]
        {
            use rustix::fs::{openat, Mode, OFlags};

            let mut components = rel.components().peekable();
            if components.peek().is_none() {
                return Err(StoreError::Other("empty indexed path".into()));
            }
            let mut directory = None;
            while let Some(component) = components.next() {
                let Component::Normal(component) = component else {
                    return Err(StoreError::Other(format!(
                        "non-relative indexed path: {}",
                        rel.display()
                    )));
                };
                let last = components.peek().is_none();
                let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
                if last {
                    // Avoid blocking if a validated regular file is swapped for
                    // a FIFO before openat. O_NONBLOCK is ignored for regular files.
                    flags |= OFlags::NONBLOCK;
                } else {
                    flags |= OFlags::DIRECTORY;
                }
                let opened = match directory.as_ref() {
                    Some(fd) => openat(fd, component, flags, Mode::empty()),
                    None => openat(&self.fd, component, flags, Mode::empty()),
                }
                .map_err(std::io::Error::from)?;
                if last {
                    return read_open_file_capped(File::from(opened), rel, max_bytes);
                }
                directory = Some(opened);
            }
            unreachable!("non-empty component list returns on its last item")
        }
        #[cfg(windows)]
        {
            use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};

            if rel.as_os_str().is_empty()
                || rel
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(StoreError::Other(format!(
                    "non-relative indexed path: {}",
                    rel.display()
                )));
            }
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            // cap-std resolves each component relative to held directory handles
            // on Windows, preventing junction replacement from escaping the root.
            let file = self.dir.open_with(rel, &options)?.into_std();
            read_open_file_capped(file, rel, max_bytes)
        }
        #[cfg(not(any(unix, windows)))]
        {
            if rel.as_os_str().is_empty()
                || rel
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(StoreError::Other(format!(
                    "non-relative indexed path: {}",
                    rel.display()
                )));
            }
            // Unsupported non-Unix/non-Windows targets lack the platform
            // handle-relative implementation above. Canonicalize and confine
            // before opening; this is safe against static escapes, though not
            // equivalent to the supported platforms' race-free resolution.
            let path = self.path.join(rel).canonicalize()?;
            if !path.starts_with(&self.path) {
                return Err(StoreError::Other(format!(
                    "indexed path escaped project root: {}",
                    rel.display()
                )));
            }
            read_text_capped(&path, max_bytes)
        }
    }
}

/// Maximum bytes accepted by `index_file` / walk indexing (64 MiB).
pub const MAX_INDEX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Result of reading one bounded newline-delimited input record.
pub enum BoundedLine {
    Line(Vec<u8>),
    TooLong,
}

/// Read and drain one newline-delimited record while retaining at most
/// `limit + 1` bytes. Unlike [`BufRead::lines`], this rejects hostile
/// unterminated input without first allocating the complete record.
pub fn read_bounded_line(
    reader: &mut impl BufRead,
    limit: usize,
) -> std::io::Result<Option<BoundedLine>> {
    let mut line = Vec::with_capacity(limit.min(8 * 1024));
    let mut too_long = false;
    let mut saw_input = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if saw_input {
                Ok(Some(if too_long {
                    BoundedLine::TooLong
                } else {
                    BoundedLine::Line(line)
                }))
            } else {
                Ok(None)
            };
        }
        saw_input = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let payload_len = newline.unwrap_or(available.len());
        if !too_long {
            let remaining = limit.saturating_add(1).saturating_sub(line.len());
            line.extend_from_slice(&available[..payload_len.min(remaining)]);
            too_long = line.len() > limit || payload_len > remaining;
        }
        reader.consume(consumed);
        if newline.is_some() {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(if too_long {
                BoundedLine::TooLong
            } else {
                BoundedLine::Line(line)
            }));
        }
    }
}

/// Read a UTF-8 source file with an explicit size cap.
pub fn read_text_capped(path: &Path, max_bytes: u64) -> Result<String> {
    Ok(read_open_file_capped(File::open(path)?, path, max_bytes)?.text)
}

fn read_open_file_capped(
    mut file: File,
    display_path: &Path,
    max_bytes: u64,
) -> Result<CappedText> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(StoreError::Other(format!(
            "indexed path is not a regular file: {}",
            display_path.display()
        )));
    }
    if metadata.len() > max_bytes {
        return Err(StoreError::Other(format!(
            "file exceeds {} byte index cap: {}",
            max_bytes,
            display_path.display()
        )));
    }
    let mut buf = String::new();
    // Cap the read even if the file grows between stat and read.
    file.by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_string(&mut buf)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidData {
                StoreError::Other(format!("binary file: {}", display_path.display()))
            } else {
                StoreError::Io(e)
            }
        })?;
    if buf.len() as u64 > max_bytes {
        return Err(StoreError::Other(format!(
            "file exceeds {} byte index cap: {}",
            max_bytes,
            display_path.display()
        )));
    }
    Ok(CappedText {
        text: buf,
        metadata,
    })
}

