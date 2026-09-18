//! Target guards shared by plan and apply: index-path confinement and the
//! owner-writability predicate both phases fold to.

use anyhow::bail;
use std::fs;
use std::path::{Component, Path};

pub(crate) fn confined_relative_path(path: &str) -> anyhow::Result<&Path> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("index contains a non-relative path: {}", path.display());
    }
    Ok(path)
}

/// A planned target the owner cannot write must be refused, not
/// rewritten. The reference's update-all refuses the same state per file
/// (`Cannot rewrite file …` / `Permission denied`, os error 13) and leaves
/// it byte+mode intact. On unix the owner-write bit is the permission that
/// matters (the process runs as the owner; the staged-rename commit would
/// otherwise bypass a clear write bit); other platforms fall back to the
/// read-only predicate.
///
/// The codemod path touches two permission representations — `std` for the
/// plan-time capped reads (`CappedText::metadata`, captured at file open)
/// and cap-std for the apply-time staging and swap-time re-verification
/// (fresh fstatat each call) — so the predicate is expressed once over both.
/// Neither representation caches the mode; plan/apply never share a metadata
/// value.
pub(crate) trait TargetWritability {
    fn owner_cannot_write(&self) -> bool;
}

impl TargetWritability for fs::Permissions {
    fn owner_cannot_write(&self) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            self.mode() & 0o200 == 0
        }
        #[cfg(not(unix))]
        {
            self.readonly()
        }
    }
}

impl TargetWritability for cap_std::fs::Permissions {
    fn owner_cannot_write(&self) -> bool {
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;
            PermissionsExt::mode(self) & 0o200 == 0
        }
        #[cfg(not(unix))]
        {
            self.readonly()
        }
    }
}

pub(crate) fn target_refuses_writes<P: TargetWritability>(permissions: &P) -> bool {
    permissions.owner_cannot_write()
}
