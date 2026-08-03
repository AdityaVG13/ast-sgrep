use super::embed_support::normalize_rel;
use super::sql::optional_row;
use super::sqlite::IndexStore;
use crate::Result;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

impl IndexStore {
    pub fn resolve_module_path(&self, from_file: &str, module: &str) -> Result<Vec<String>> {
        let module = module.trim().trim_matches(['"', '\'']);
        if module.is_empty() {
            return Ok(Vec::new());
        }
        let lang = self.file_language(from_file)?;
        let parent = Path::new(from_file)
            .parent()
            .unwrap_or_else(|| Path::new(""));
        let bases = match lang.as_deref() {
            Some("python") => resolve_bases_python(parent, module),
            Some("javascript") | Some("typescript") => resolve_bases_js(parent, module),
            Some("go") => resolve_bases_go(from_file, parent, module),
            // Rust (default): :: paths, crate/super/self, /src/ layout.
            _ => resolve_bases_rust(from_file, parent, module),
        };
        let exts: &[&str] = match lang.as_deref() {
            Some("python") => &["py"],
            Some("javascript") => &["js", "jsx", "mjs", "cjs"],
            Some("typescript") => &["ts", "tsx", "js", "jsx"],
            Some("go") => &["go"],
            Some("rust") => &["rs"],
            _ => &[
                "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "cs", "rb",
            ],
        };
        let mut cands = BTreeSet::new();
        for base in bases {
            let n = normalize_rel(&base);
            cands.insert(n.clone());
            if base.extension().is_none() {
                for e in exts {
                    cands.insert(format!("{n}.{e}"));
                }
                match lang.as_deref() {
                    Some("python") => {
                        cands.insert(format!("{n}/__init__.py"));
                    }
                    Some("javascript") | Some("typescript") => {
                        for e in ["ts", "tsx", "js", "jsx"] {
                            cands.insert(format!("{n}/index.{e}"));
                        }
                    }
                    Some("go") => {
                        // package dir: any .go file under the package path is matched
                        // via file_exists on exact candidates; also try package.go.
                        cands.insert(format!(
                            "{n}/{}.go",
                            base.file_name().and_then(|s| s.to_str()).unwrap_or("pkg")
                        ));
                    }
                    _ => {
                        cands.insert(format!("{n}/mod.rs"));
                        for e in ["ts", "tsx", "js", "jsx"] {
                            cands.insert(format!("{n}/index.{e}"));
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        for c in cands {
            if self.file_exists(&c)? {
                out.push(c);
            }
        }
        Ok(out)
    }

    fn file_language(&self, path: &str) -> Result<Option<String>> {
        optional_row(
            self.connection(),
            "SELECT language FROM files WHERE path=?1",
            &[&path],
            |r| r.get(0),
        )
    }
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

fn resolve_bases_python(parent: &Path, module: &str) -> Vec<PathBuf> {
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

fn resolve_bases_js(parent: &Path, module: &str) -> Vec<PathBuf> {
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
