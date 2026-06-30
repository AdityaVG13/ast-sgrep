//! Glob-style path matching for gitignore rules.

pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let pat = pattern.trim_end_matches('/');
    if pat.contains("**/") {
        if let Some(rest) = pat.split("**/").nth(1) {
            return glob_matches(rest, text)
                || text.split('/').any(|seg| glob_matches(rest, seg));
        }
    }
    if let Some(suffix) = pat.strip_prefix('*') {
        return text.ends_with(suffix) || text.split('/').any(|seg| seg.ends_with(suffix));
    }
    if let Some(prefix) = pat.strip_suffix('*') {
        return text.starts_with(prefix)
            || text.split('/').any(|seg| seg.starts_with(prefix));
    }
    text == pat || text.starts_with(&format!("{pat}/"))
}

pub fn pattern_matches(pattern: &str, path: &str, dir_only: bool) -> bool {
    let mut pat = pattern.trim_end_matches('/');
    let anchored = pat.starts_with('/');
    if anchored {
        pat = &pat[1..];
    }

    let matched = if pat.contains('/') || anchored {
        glob_matches(pat, path)
    } else {
        path.split('/').any(|seg| glob_matches(pat, seg)) || glob_matches(pat, path)
    };

    matched || (dir_only && (path == pat || path.starts_with(&format!("{pat}/"))))
}
