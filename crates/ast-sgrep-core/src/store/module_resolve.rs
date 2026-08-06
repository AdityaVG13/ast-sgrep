//! Language-specific module path resolution for import graph expansion.

use super::embed_support::normalize_rel;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Collect candidate relative paths for a module import (existence checked by caller).
pub(crate) fn collect_module_candidates(
    from_file: &str,
    module: &str,
    lang: Option<&str>,
) -> BTreeSet<String> {
    let module = module.trim().trim_matches(['"', '\'']);
    if module.is_empty() {
        return BTreeSet::new();
    }
    let parent = Path::new(from_file)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let rules = module_resolve_rules(lang);
    let bases = (rules.bases)(from_file, parent, module);
    let mut cands = BTreeSet::new();
    for base in bases {
        let n = normalize_rel(&base);
        cands.insert(n.clone());
        if base.extension().is_none() {
            for e in rules.exts {
                cands.insert(format!("{n}.{e}"));
            }
            (rules.add_extras)(&mut cands, &n, &base);
        }
    }
    cands
}

fn resolve_bases_rust(from_file: &str, parent: &Path, module: &str) -> Vec<PathBuf> {
    let crate_src = from_file
        .find("/src/")
        .map(|i| Path::new(&from_file[..i + 4]));
    let slash = module.replace("::", "/");
    let mut bases = Vec::new();
    if let Some(rest) = slash.strip_prefix("crate/") {
        if let Some(src) = crate_src {
            bases.push(src.join(rest));
        }
    } else if slash == "crate" {
        if let Some(src) = crate_src {
            bases.push(src.to_path_buf());
        }
    } else if slash.starts_with("super/") || slash.starts_with("self/") {
        let mut base = parent.to_path_buf();
        let mut rest = slash.as_str();
        while let Some(n) = rest.strip_prefix("super/") {
            base.pop();
            rest = n;
        }
        rest = rest.strip_prefix("self/").unwrap_or(rest);
        bases.push(base.join(rest));
    } else if module.starts_with('.') {
        bases.push(parent.join(module));
    } else {
        bases.push(parent.join(&slash));
        if let Some(src) = crate_src {
            bases.push(src.join(&slash));
        }
    }
    bases
}

/// Language → {extensions, base resolver, package-style extras}. Candidate sets must stay
/// identical to the prior match arms (BTreeSet order is key-ordered).
struct ModuleResolveRules {
    exts: &'static [&'static str],
    bases: fn(&str, &Path, &str) -> Vec<PathBuf>,
    add_extras: fn(&mut BTreeSet<String>, &str, &Path),
}

const JS_INDEX_EXTS: &[&str] = &["ts", "tsx", "js", "jsx"];
const DEFAULT_MODULE_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "cs", "rb",
];

fn extras_python(cands: &mut BTreeSet<String>, n: &str, _base: &Path) {
    cands.insert(format!("{n}/__init__.py"));
}
fn extras_js_ts(cands: &mut BTreeSet<String>, n: &str, _base: &Path) {
    for e in JS_INDEX_EXTS {
        cands.insert(format!("{n}/index.{e}"));
    }
}
fn extras_go(cands: &mut BTreeSet<String>, n: &str, base: &Path) {
    // package dir: any .go file under the package path is matched
    // via file_exists on exact candidates; also try package.go.
    cands.insert(format!(
        "{n}/{}.go",
        base.file_name().and_then(|s| s.to_str()).unwrap_or("pkg")
    ));
}
fn extras_default_rustish(cands: &mut BTreeSet<String>, n: &str, _base: &Path) {
    cands.insert(format!("{n}/mod.rs"));
    for e in JS_INDEX_EXTS {
        cands.insert(format!("{n}/index.{e}"));
    }
}

fn module_resolve_rules(lang: Option<&str>) -> ModuleResolveRules {
    match lang {
        Some("python") => ModuleResolveRules {
            exts: &["py"],
            bases: resolve_bases_python,
            add_extras: extras_python,
        },
        Some("javascript") => ModuleResolveRules {
            exts: &["js", "jsx", "mjs", "cjs"],
            bases: resolve_bases_js,
            add_extras: extras_js_ts,
        },
        Some("typescript") => ModuleResolveRules {
            exts: &["ts", "tsx", "js", "jsx"],
            bases: resolve_bases_js,
            add_extras: extras_js_ts,
        },
        Some("go") => ModuleResolveRules {
            exts: &["go"],
            bases: resolve_bases_go,
            add_extras: extras_go,
        },
        Some("rust") => ModuleResolveRules {
            exts: &["rs"],
            bases: resolve_bases_rust,
            add_extras: extras_default_rustish,
        },
        // Unknown / missing language: Rust-shaped bases + broad extension probe.
        _ => ModuleResolveRules {
            exts: DEFAULT_MODULE_EXTS,
            bases: resolve_bases_rust,
            add_extras: extras_default_rustish,
        },
    }
}

fn resolve_bases_python(_from_file: &str, parent: &Path, module: &str) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if module.starts_with('.') {
        let dots = module.chars().take_while(|c| *c == '.').count();
        let cleaned = module.trim_start_matches('.');
        let mut base = parent.to_path_buf();
        // PEP 328: one leading dot = current package; each extra pops one level.
        for _ in 1..dots {
            base.pop();
        }
        if cleaned.is_empty() {
            bases.push(base);
        } else {
            bases.push(base.join(cleaned.replace('.', "/")));
        }
    } else {
        let slash = module.replace('.', "/");
        bases.push(PathBuf::from(&slash));
        let mut cur = parent.to_path_buf();
        for _ in 0..6 {
            bases.push(cur.join(&slash));
            if !cur.pop() {
                break;
            }
        }
    }
    bases
}

fn resolve_bases_js(_from_file: &str, parent: &Path, module: &str) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if module.starts_with('.') {
        bases.push(parent.join(module));
    } else {
        // Bare specifier: walk up for node_modules/<name>, plus same-dir fallback.
        bases.push(parent.join(module));
        let mut cur = parent.to_path_buf();
        for _ in 0..8 {
            bases.push(cur.join("node_modules").join(module));
            if !cur.pop() {
                break;
            }
        }
    }
    bases
}

fn resolve_bases_go(from_file: &str, parent: &Path, module: &str) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if module.starts_with('.') {
        bases.push(parent.join(module));
        return bases;
    }
    let parts: Vec<&str> = module.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return bases;
    }
    // Match local packages by import-path suffix against indexed tree roots.
    for n in 1..=parts.len().min(4) {
        let suffix = parts[parts.len() - n..].join("/");
        bases.push(PathBuf::from(&suffix));
        let mut cur = parent.to_path_buf();
        for _ in 0..6 {
            bases.push(cur.join(&suffix));
            if !cur.pop() {
                break;
            }
        }
        // Also try beside the importing file's module root (first path segment).
        if let Some(root) = Path::new(from_file).components().next() {
            bases.push(PathBuf::from(root.as_os_str()).join(&suffix));
        }
    }
    bases
}
