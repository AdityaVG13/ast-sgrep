use crate::query::ParsedQuery;
use crate::Result;

/// Resolve an `in:` scope against the index root BEFORE any walk. A
/// parse-time scope error refuses loudly (the
/// historic silent drop ran the query unscoped); a scope that exists on disk
/// as a FILE pins the exact file rel-path, a directory keeps the
/// `dir/**` glob, and a path matching nothing under the root refuses with a
/// diagnostic instead of answering silent-empty. Wildcard scopes bypass disk
/// resolution (their glob is the filter, existence of individual matches is
/// the walk's own business).
///
/// The filter glob is derived HERE, once, and stored back into
/// `parsed.path_scope` — finish compiles it directly with no branch and no
/// second derivation.
pub(crate) fn resolve_path_scope(root: &std::path::Path, parsed: &mut ParsedQuery) -> Result<()> {
    if let Some(err) = parsed.path_scope_error.as_deref() {
        return Err(crate::StoreError::Other(err.to_string()));
    }
    let Some(scope) = parsed.path_scope.as_deref() else {
        return Ok(());
    };
    let has_wildcard = scope.contains('*') || scope.contains('?');
    let mut is_exact = false;
    if !has_wildcard {
        // Check the authored path before lexical normalization: file/ must
        // still refuse instead of being changed into an admitted file.
        let target = root.join(scope);
        if target.is_file() {
            is_exact = true;
        } else if !target.is_dir() {
            return Err(crate::StoreError::Other(format!(
                "in: scope '{scope}' matches no file or directory under {}",
                root.display()
            )));
        }
    }
    // Disk admission and hit filtering must use the same relative identity.
    // Deliberately NOT `paths::canonicalize_path` (either policy): following
    // symlinks would rewrite authored glob components or literal Unix
    // backslash filenames. Lexical normalization is the policy here.
    let relative: std::path::PathBuf = std::path::Path::new(scope)
        .components()
        .filter(|component| !matches!(component, std::path::Component::CurDir))
        .collect();
    let mut normalized = relative
        .to_str()
        .expect("components of a UTF-8 query scope remain UTF-8")
        .replace(std::path::MAIN_SEPARATOR, "/");
    if normalized.is_empty() {
        normalized.push_str("**");
    } else if scope.ends_with('/') || scope.ends_with("/.") {
        normalized.push('/');
    }
    // Exact files keep the bare rel-path filter; everything else expands
    // through the single `path_scope_glob` helper.
    parsed.path_scope = Some(if is_exact {
        normalized
    } else {
        crate::query::path_scope_glob(&normalized)
    });
    Ok(())
}

pub(crate) fn compile_glob(pattern: &str) -> std::result::Result<regex::Regex, String> {
    if pattern.is_empty() {
        return Err("file_filter must be non-empty".into());
    }
    if pattern.chars().count() > crate::limits::MAX_FILE_FILTER_CHARS {
        return Err(format!(
            "file_filter exceeds maximum of {} characters",
            crate::limits::MAX_FILE_FILTER_CHARS
        ));
    }
    if pattern
        .chars()
        .any(|c| c == '\0' || (c.is_control() && c != '\t'))
    {
        return Err("file_filter contains invalid control characters".into());
    }
    let mut result = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    result.push_str("(?:.*/)?");
                } else {
                    result.push_str(".*");
                }
            }
            '*' => result.push_str(".*"),
            '?' => result.push('.'),
            c if "\\.+()|[]{}^$".contains(c) => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result.push('$');
    regex::Regex::new(&result).map_err(|e| e.to_string())
}

pub(crate) fn validate_query_arg(query: &str) -> Result<()> {
    crate::limits::validate_query_len(query).map_err(crate::StoreError::Other)
}
