//! File URI encoding/decoding (RFC 8089) with workspace-relative resolution.

use std::path::{Component, Path, PathBuf};

/// Convert a filesystem path to a `file:` URI with percent-encoding.
pub fn path_to_file_uri(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path_str = canonical.to_string_lossy().replace('\\', "/");
    if path_str.len() >= 2 && path_str.as_bytes()[1] == b':' {
        format!("file:///{}", percent_encode_path(&path_str))
    } else if path_str.starts_with('/') {
        format!("file://{}", percent_encode_path(&path_str))
    } else {
        format!("file:///{}", percent_encode_path(&path_str))
    }
}

/// Decode a `file:` URI to an absolute filesystem path.
pub fn file_uri_to_path(uri: &str) -> anyhow::Result<PathBuf> {
    let rest = uri
        .strip_prefix("file://")
        .or_else(|| uri.strip_prefix("file:"))
        .ok_or_else(|| anyhow::anyhow!("not a file URI: {uri}"))?;

    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let decoded = percent_decode(rest);
    let path_str = if rest.starts_with('/') && decoded.len() >= 2 && decoded.as_bytes()[1] == b':' {
        // file:///C:/... on Windows
        decoded.trim_start_matches('/').to_string()
    } else {
        decoded
    };
    Ok(PathBuf::from(path_str))
}

/// Resolve a document URI to a path relative to the canonical workspace root.
pub fn uri_to_rel_path(uri: &str, root: &Path) -> anyhow::Result<String> {
    let abs = file_uri_to_path(uri)?;
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canonical_abs = abs.canonicalize().unwrap_or(abs);
    if !canonical_abs.starts_with(&canonical_root) {
        anyhow::bail!("document URI outside workspace root");
    }
    let rel = canonical_abs
        .strip_prefix(&canonical_root)
        .map_err(|_| anyhow::anyhow!("document URI outside workspace root"))?;
    if rel.components().any(|c| matches!(c, Component::ParentDir)) {
        anyhow::bail!("path traversal in document URI");
    }
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

/// Canonical workspace root from LSP initialize params.
pub fn canonicalize_workspace_root(root: PathBuf) -> PathBuf {
    root.canonicalize().unwrap_or(root)
}

fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b':' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
