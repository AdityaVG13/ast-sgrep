//! Statement/declaration root-template lanes.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

// ===========================================================================
// Statement/decl-root lanes answering where the general lane stays silent:
//   rs `mod $N { $B }` (single-statement-exact body) / `extern crate $N;`
//   go `type $N $T` (T = the whole type tail)
//   rb `module $N\n  $B\nend` (B = the trimmed body)
//   ts `declare module $N { $B }` (single-statement-exact) / `declare const $X: $T;`
//   py `async def $N($$P):\n    $$B` (P param-exact, B = the whole body block)
//   java `enum <name> { <body> }` (single-member-exact; empty-body pattern
//        binds empty candidates)
//   cs `record <name>(<P>);[ { $B }]` / `struct <name> { $B }`
// Each lane's own parse is the admission. Modifier doctrine: a candidate
// carrying a modifier/visibility child the pattern lacks REFUSES.
// ===========================================================================

#[derive(Debug, Clone)]
pub(crate) enum Root142Template {
    RsMod {
        name: LaneName,
        body: LaneMeta,
    },
    RsExternCrate {
        name: LaneName,
    },
    GoType {
        name: LaneName,
        ty: LaneName,
    },
    RbModule {
        name: LaneName,
        body: LaneMeta,
    },
    TsDeclareModule {
        name: LaneName,
        body: LaneMeta,
    },
    TsDeclareConst {
        name: LaneName,
        ty: LaneName,
    },
    PyAsyncDef {
        name: LaneName,
        param: LaneMeta,
        body: LaneMeta,
    },
    JaEnum {
        name: LaneName,
        body: JaEnumBody,
    },
    CsRecord {
        name: LaneName,
        param: LaneMeta,
        body: Option<LaneMeta>,
    },
    CsStruct {
        name: LaneName,
        body: LaneMeta,
    },
    /// cs `goto $L;` / semi-less `goto $L` — the reference binds PER SITE
    /// across the gap-junk class and comments. The registered c goto lane is
    /// c-scoped. Label is META-only: the concrete `goto end;` spelling keeps
    /// its literal-lane route.
    CsGoto {
        label: LaneMeta,
    },
    /// cs `new $T($A)[;]` — the reference binds the ONE-argument
    /// object-creation STATEMENT and refuses nested, zero-arg, and two-arg
    /// faces.
    CsNew {
        ty: LaneName,
        arg: LaneMeta,
    },
    /// cs switch-STATEMENT meta body — both spellings bind ONE section; empty
    /// and multi-section refuse; the switch-EXPRESSION row never crosses
    /// (kind-exact walk on `switch_statement`).
    CsSwitch {
        subject: Option<LaneName>,
        body: LaneMeta,
    },
    /// php declaration-kind meta bodies — function/class/trait/interface with
    /// a `PhpNamespaceBody` slot (Meta binds ONE member; Mixed exact-order
    /// binds); multi-member/empty refuse. The served function-`$B` face keeps
    /// its decl-lane route, so the function body DEMANDS `$$`; `$$$B`
    /// refuses (template refuses).
    PhpDecl {
        kind: PhpDeclKind,
        name: LaneName,
        body: PhpNamespaceBody,
    },
    /// go plain `switch [<X>] { <B> }` — ONE clause binds, 2-clause/empty
    /// refuse; the type-switch spelling is a distinct kind.
    GoSwitch {
        subject: Option<LaneName>,
        body: LaneMeta,
    },
    /// go `if <C> { <B> }` — B binds the WHOLE body text (single and
    /// multi); the empty body refuses. The single-`$` body keeps its served
    /// general-lane route, so the body DEMANDS `$$` (the E9 discipline).
    GoIf {
        cond: LaneName,
        body: LaneMeta,
    },
    /// go `package <N>` — binds through a comment gap (the package root was
    /// never admitted).
    GoPackage {
        name: LaneMeta,
    },
    /// go SEMI-LESS `import <X>` — X binds the single path (`"fmt"`) or the
    /// whole group text; the `;`-ful spelling keeps its registered
    /// accepted-empty envelope.
    GoImport {
        target: LaneMeta,
    },
    /// js `$$` arm bodies — see [`JsIfTemplate`].
    JsElseIf(JsIfTemplate),
}

/// The php declaration kind of a [`Root142Template::PhpDecl`] — the walk keys
/// the candidate node kind and the Mixed-doc wrapper on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhpDeclKind {
    Function,
    Class,
    Trait,
    Interface,
}

impl PhpDeclKind {
    pub(crate) fn keyword(self) -> &'static str {
        match self {
            PhpDeclKind::Function => "function",
            PhpDeclKind::Class => "class",
            PhpDeclKind::Trait => "trait",
            PhpDeclKind::Interface => "interface",
        }
    }

    pub(crate) fn root_kind(self) -> &'static str {
        match self {
            PhpDeclKind::Function => "function_definition",
            PhpDeclKind::Class => "class_declaration",
            PhpDeclKind::Trait => "trait_declaration",
            PhpDeclKind::Interface => "interface_declaration",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum JaEnumBody {
    Meta(LaneMeta),
    Literal(String),
    Empty,
}

/// One identifier token (or canonical meta) as a lane name slot.
pub(crate) fn root142_name_slot(token: &str) -> Option<LaneName> {
    if let Some(meta) = lane_meta(token) {
        return Some(LaneName::Meta(meta));
    }
    is_pattern_ident(token).then(|| LaneName::Literal(token.to_string()))
}

pub(crate) fn root142_meta_slot(token: &str) -> Option<LaneMeta> {
    lane_meta(token)
}

pub(crate) fn root142_trivia(text: &str) -> &str {
    text.trim()
}

pub(crate) fn statement_root_142_template(
    lang: Language,
    pattern: &str,
) -> Option<Root142Template> {
    let p = root142_trivia(pattern);
    match lang {
        Language::Rust => {
            // `mod <name> { $B }` — the single-statement body law lives in
            // the walk (E2: 2-statement bodies refuse).
            if let Some(rest) = p.strip_prefix("mod ") {
                let (name_tok, rest) = root142_split_token(rest)?;
                let inner = rest.trim_start().strip_prefix('{')?;
                let close = balanced_brace_close(inner)?;
                if !inner[close + 1..].trim().is_empty() {
                    return None;
                }
                let body = root142_meta_slot(inner[..close].trim())?;
                return Some(Root142Template::RsMod {
                    name: root142_name_slot(name_tok)?,
                    body,
                });
            }
            // `extern crate <name>;`
            let rest = p.strip_prefix("extern crate ")?;
            let (name_tok, rest) = root142_split_token(rest)?;
            if !root142_trivia(rest).strip_suffix(';')?.is_empty() {
                return None;
            }
            Some(Root142Template::RsExternCrate {
                name: root142_name_slot(name_tok)?,
            })
        }
        Language::Go => {
            // `type <name> <T>` — T is the WHOLE type tail (E6 struct blocks);
            // the `=`-alias spelling keeps the kt/general routes (unprobed
            // here on purpose), and the `;`-ful spelling is a reference RC8.
            if let Some(rest) = p.strip_prefix("type ") {
                let (name_tok, rest) = root142_split_token(rest)?;
                let ty = rest.trim();
                if ty.is_empty() || ty.starts_with('=') || ty.contains(';') {
                    return None;
                }
                if ty.contains(char::is_whitespace) {
                    // A multi-token tail never parses in go without a brace
                    // group; the brace-group tail is unprobed and keeps its
                    // loud route.
                    return None;
                }
                return Some(Root142Template::GoType {
                    name: root142_name_slot(name_tok)?,
                    ty: root142_name_slot(ty)?,
                });
            }
            // The go root siblings of the goto-bare face.
            if let Some(rest) = p.strip_prefix("switch ") {
                // `switch [<X>] { <B> }` — the global `{` form has no subject.
                let (subject, brace_section) = if let Some(inner) = rest.strip_prefix('{') {
                    (None, inner)
                } else {
                    let (subj_tok, after) = root142_split_token(rest)?;
                    let brace_at = after.find('{')?;
                    if !after[..brace_at].trim().is_empty() {
                        return None;
                    }
                    (Some(root142_name_slot(subj_tok)?), &after[brace_at + 1..])
                };
                let close = balanced_brace_close(brace_section)?;
                if !brace_section[close + 1..].trim().is_empty() {
                    return None;
                }
                // Body spelling: `$B` and `$$B` both bind the ONE clause's
                // raw text; `$$$B` is unprobed and keeps its prior class.
                let body_tok = brace_section[..close].trim();
                if body_tok.starts_with("$$$") {
                    return None;
                }
                let body = root142_meta_slot(body_tok)?;
                return Some(Root142Template::GoSwitch { subject, body });
            }
            if let Some(rest) = p.strip_prefix("if ") {
                // `if <C> { <B> }` — C is one token slot (e13 `x` literal,
                // e10 `$C` meta); B binds the WHOLE body and demands `$$`.
                let brace_at = rest.find('{')?;
                let cond_tok = rest[..brace_at].trim();
                if cond_tok.contains(char::is_whitespace) {
                    return None;
                }
                let cond = root142_name_slot(cond_tok)?;
                let inner = &rest[brace_at + 1..];
                let close = balanced_brace_close(inner)?;
                if !inner[close + 1..].trim().is_empty() {
                    return None;
                }
                let body_tok = inner[..close].trim();
                if !body_tok.starts_with("$$") || body_tok.starts_with("$$$") {
                    return None;
                }
                let body = root142_meta_slot(body_tok)?;
                return Some(Root142Template::GoIf { cond, body });
            }
            if let Some(rest) = p.strip_prefix("package ") {
                // `package <N>` — meta name, no semi (e5/e6).
                let tok = rest.trim();
                if tok.contains(char::is_whitespace) || tok.ends_with(';') {
                    return None;
                }
                let name = lane_meta(tok)?;
                if name.multi {
                    return None;
                }
                return Some(Root142Template::GoPackage { name });
            }
            if let Some(rest) = p.strip_prefix("import ") {
                // SEMI-LESS `import <X>` (e8/e9); the `;`-ful spelling keeps
                // its registered accepted-empty envelope upstream.
                let tok = rest.trim();
                if tok.contains(char::is_whitespace) || tok.ends_with(';') {
                    return None;
                }
                let target = lane_meta(tok)?;
                if target.multi {
                    return None;
                }
                return Some(Root142Template::GoImport { target });
            }
            None
        }
        Language::Ruby => {
            // `module <Name>\n  $B\nend` — the body slot is the whole
            // trimmed middle; only the canonical-meta spelling is probed.
            let rest = p.strip_prefix("module ")?;
            let (name_tok, rest) = root142_split_token(rest)?;
            let mid = rest.trim_end().strip_suffix("end")?;
            if !mid.trim_end().is_empty() && !mid.ends_with(|c: char| c.is_whitespace()) {
                // `end` must be its own token (an `endX` tail is a name).
                return None;
            }
            Some(Root142Template::RbModule {
                name: root142_name_slot(name_tok)?,
                body: root142_meta_slot(mid.trim())?,
            })
        }
        Language::TypeScript => {
            if let Some(rest) = p.strip_prefix("declare module ") {
                let (name_tok, rest) = root142_split_token(rest)?;
                let inner = rest.trim_start().strip_prefix('{')?;
                let close = balanced_brace_close(inner)?;
                if !inner[close + 1..].trim().is_empty() {
                    return None;
                }
                let body = root142_meta_slot(inner[..close].trim())?;
                return Some(Root142Template::TsDeclareModule {
                    name: root142_name_slot(name_tok).or_else(|| {
                        // The quoted ambient name (`"m"`) is a literal slot.
                        let tok = root142_trivia(name_tok);
                        (!tok.is_empty()
                            && !tok.contains('$')
                            && !tok.contains(char::is_whitespace))
                        .then(|| LaneName::Literal(tok.to_string()))
                    })?,
                    body,
                });
            }
            // `declare const <X>: <T>;` — the kind token is part of the FACE
            // (H23: `declare let` refuses).
            let rest = p.strip_prefix("declare const ")?;
            let (name_tok, rest) = root142_split_token(rest)?;
            let ty = root142_trivia(rest).strip_suffix(';')?;
            let ty = ty.strip_prefix(':').unwrap_or(ty).trim();
            if ty.is_empty() || ty.contains(char::is_whitespace) {
                return None;
            }
            Some(Root142Template::TsDeclareConst {
                name: root142_name_slot(name_tok)?,
                ty: root142_name_slot(ty)?,
            })
        }
        Language::Python => py_async_def_template(pattern).map(|t| match t {
            PyAsyncDefTemplate { name, param, body } => {
                Root142Template::PyAsyncDef { name, param, body }
            }
        }),
        Language::Java => {
            // `enum <name> { <body> }` — the body is one canonical meta, one
            // literal token, or EMPTY (K4: the empty pattern binds empty
            // candidates). Name meta-or-literal.
            let rest = p.strip_prefix("enum ")?;
            let (name_tok, rest) = root142_split_token(rest)?;
            let inner = rest.trim_start().strip_prefix('{')?;
            let close = balanced_brace_close(inner)?;
            if !inner[close + 1..].trim().is_empty() {
                return None;
            }
            let body_text = inner[..close].trim();
            let body = if body_text.is_empty() {
                JaEnumBody::Empty
            } else if let Some(meta) = root142_meta_slot(body_text) {
                JaEnumBody::Meta(meta)
            } else if is_pattern_ident(body_text) {
                JaEnumBody::Literal(body_text.to_string())
            } else {
                return None;
            };
            Some(Root142Template::JaEnum {
                name: root142_name_slot(name_tok)?,
                body,
            })
        }
        Language::CSharp => {
            if let Some(rest) = p.strip_prefix("record ") {
                // `record <name>(<P>);` optionally ` { $B }`.
                let (name_tok, rest) = root142_split_token(rest)?;
                let params_inner = rest.trim_start().strip_prefix('(')?;
                let close = balanced_paren_close(params_inner)?;
                let after = &params_inner[close + 1..];
                let param = root142_meta_slot(params_inner[..close].trim())?;
                let after_trim = after.trim();
                if after_trim.is_empty() || after_trim == ";" {
                    return Some(Root142Template::CsRecord {
                        name: root142_name_slot(name_tok)?,
                        param,
                        body: None,
                    });
                }
                let brace_at = after.find('{')?;
                let pre_brace = after[..brace_at].trim();
                if !pre_brace.is_empty() && pre_brace != ";" {
                    return None;
                }
                let inner = &after[brace_at + 1..];
                let close = balanced_brace_close(inner)?;
                if !inner[close + 1..].trim().is_empty() && inner[close + 1..].trim() != ";" {
                    return None;
                }
                return Some(Root142Template::CsRecord {
                    name: root142_name_slot(name_tok)?,
                    param,
                    body: root142_meta_slot(inner[..close].trim()).map(Some)?,
                });
            }
            // `struct <name> { $B }`.
            if let Some(rest) = p.strip_prefix("struct ") {
                let (name_tok, rest) = root142_split_token(rest)?;
                let inner = rest.trim_start().strip_prefix('{')?;
                let close = balanced_brace_close(inner)?;
                if !inner[close + 1..].trim().is_empty() {
                    return None;
                }
                return Some(Root142Template::CsStruct {
                    name: root142_name_slot(name_tok)?,
                    body: root142_meta_slot(inner[..close].trim())?,
                });
            }
            if p.split_whitespace().next() == Some("goto") {
                // `goto <L>[;]` — meta label only; the concrete-label spelling
                // keeps its literal route.
                let rest = p[4..].trim();
                let rest = rest.strip_suffix(';').unwrap_or(rest).trim();
                if rest.is_empty() || rest.contains(char::is_whitespace) {
                    return None;
                }
                let label = lane_meta(rest)?;
                if label.multi {
                    return None;
                }
                return Some(Root142Template::CsGoto { label });
            }
            if p.split_whitespace().next() == Some("new") {
                // `new <T>(<A>)[;]` — T meta or literal ident; A one
                // single-meta argument (zero/two-arg faces refuse).
                let rest = p[3..].trim_start();
                let (ty_tok, rest) = root142_split_token(rest)?;
                let params = rest.trim_start().strip_prefix('(')?;
                let close = balanced_paren_close(params)?;
                let arg = params[..close].trim();
                let mut after = params[close + 1..].trim();
                if let Some(stripped) = after.strip_suffix(';') {
                    after = stripped.trim_end();
                }
                if !after.is_empty() {
                    return None;
                }
                let arg = lane_meta(arg)?;
                if arg.multi {
                    return None;
                }
                return Some(Root142Template::CsNew {
                    ty: root142_name_slot(ty_tok)?,
                    arg,
                });
            }
            if p.split_whitespace().next() == Some("switch") {
                // `switch (<X>) { <B> }` — the subject is a single
                // meta-or-literal token slot; B binds ONE section (both
                // spellings).
                let rest = p[6..].trim_start();
                let paren_inner = rest.strip_prefix('(')?;
                let close = balanced_paren_close(paren_inner)?;
                let subject = root142_name_slot(paren_inner[..close].trim())?;
                let inner = paren_inner[close + 1..].trim_start().strip_prefix('{')?;
                let close_b = balanced_brace_close(inner)?;
                if !inner[close_b + 1..].trim().is_empty() {
                    return None;
                }
                let body = root142_meta_slot(inner[..close_b].trim())?;
                if body.multi {
                    return None;
                }
                return Some(Root142Template::CsSwitch {
                    subject: Some(subject),
                    body,
                });
            }
            None
        }
        Language::JavaScript => {
            // The js `$$`-arm faces — see [`js_if_meta_template`]. The served
            // single-`$` faces keep their routes through the `$$` demand.
            js_if_meta_template(p).map(Root142Template::JsElseIf)
        }
        Language::Php => {
            // The declaration-kind meta-body faces. The function body demands
            // `$$` (the served `$B` face keeps its decl-lane route);
            // class/trait/interface admit both spellings.
            php_decl_block_template(p)
        }
        _ => None,
    }
}

/// The php declaration-kind prefixes and the kind each introduces. The
/// spellings share no leading character, so table order cannot shadow.
const PHP_DECL_PREFIXES: &[(&str, PhpDeclKind)] = &[
    ("function ", PhpDeclKind::Function),
    ("class ", PhpDeclKind::Class),
    ("trait ", PhpDeclKind::Trait),
    ("interface ", PhpDeclKind::Interface),
];

/// The php declaration-kind template — `function <N>() { <B> }` /
/// `<class|trait|interface> <N> { <B> }`. B is a [`PhpNamespaceBody`] (Meta
/// ONE-member or Mixed exact-order). `$$$B` stays refused; the function body
/// demands `$$` (the served `$B` face keeps its decl-lane route).
pub(crate) fn php_decl_block_template(pattern: &str) -> Option<Root142Template> {
    let p = pattern.trim();
    let (kind, rest) = PHP_DECL_PREFIXES
        .iter()
        .find_map(|(prefix, kind)| p.strip_prefix(prefix).map(|rest| (*kind, rest)))?;
    let (name_tok, rest) = root142_split_token(rest)?;
    if kind == PhpDeclKind::Function {
        // The parameter list is part of the probed face: empty `()` (c1).
        let params = rest.trim_start();
        let inner = params.strip_prefix('(')?;
        let close = balanced_paren_close(inner)?;
        if !inner[..close].trim().is_empty() {
            return None;
        }
        let _ = params;
        let rest = inner[close + 1..].trim_start();
        let inner = rest.strip_prefix('{')?;
        let close_b = balanced_brace_close(inner)?;
        if !inner[close_b + 1..].trim().is_empty() {
            return None;
        }
        let body = php_decl_body_slot(inner[..close_b].trim(), true)?;
        return Some(Root142Template::PhpDecl {
            kind,
            name: root142_name_slot(name_tok)?,
            body,
        });
    }
    let inner = rest.trim_start().strip_prefix('{')?;
    let close = balanced_brace_close(inner)?;
    if !inner[close + 1..].trim().is_empty() {
        return None;
    }
    let body = php_decl_body_slot(inner[..close].trim(), false)?;
    Some(Root142Template::PhpDecl {
        kind,
        name: root142_name_slot(name_tok)?,
        body,
    })
}

/// The php declaration body slot: canonical meta (`$B`/`$$B`) or the Mixed
/// exact-order face; `$$$B` refuses (unprobed). `demand_dollar_dollar`
/// scopes the served `$B` face out (function bodies).
pub(crate) fn php_decl_body_slot(
    section: &str,
    demand_dollar_dollar: bool,
) -> Option<PhpNamespaceBody> {
    let body = if let Some(meta) = lane_meta(section) {
        if meta.multi {
            return None;
        }
        if demand_dollar_dollar && !section.trim().starts_with("$$") {
            return None;
        }
        PhpNamespaceBody::Meta(meta)
    } else if let Some((prefix, tail)) = php_mixed_namespace_body(section) {
        PhpNamespaceBody::Mixed { prefix, tail }
    } else {
        return None;
    };
    Some(body)
}

/// The js `$$`-arm template shapes: `if (<A>) { <$$B> } [else { <$$D> } |
/// else if (<C>) { <$$D> }]`. The walk emits PER LEVEL on else-if chains,
/// so the else-if tail beyond D is deliberately not consumed (a trailing
/// chain spelling refuses here).
#[derive(Debug, Clone)]
pub(crate) struct JsIfTemplate {
    cond: LaneMeta,
    body: LaneMeta,
    tail: JsTail,
}

#[derive(Debug, Clone)]
pub(crate) enum JsTail {
    None,
    Block(LaneMeta),
    ElseIf { cond: LaneMeta, body: LaneMeta },
}

pub(crate) fn js_if_meta_template(pattern: &str) -> Option<JsIfTemplate> {
    let p = pattern.trim();
    let rest = p.strip_prefix("if ")?;
    let paren_inner = rest.trim_start().strip_prefix('(')?;
    let close = balanced_paren_close(paren_inner)?;
    let cond = js_cond_slot(paren_inner[..close].trim())?;
    let rest = paren_inner[close + 1..].trim_start();
    let inner = rest.strip_prefix('{')?;
    let close_b = balanced_brace_close(inner)?;
    let body = js_arm_slot(inner[..close_b].trim())?;
    let rest = inner[close_b + 1..].trim();
    let tail = if rest.is_empty() {
        JsTail::None
    } else if let Some(rest) = rest.strip_prefix("else if ") {
        let paren_inner = rest.trim_start().strip_prefix('(')?;
        let close = balanced_paren_close(paren_inner)?;
        let cond = js_cond_slot(paren_inner[..close].trim())?;
        let rest = paren_inner[close + 1..].trim_start();
        let inner = rest.strip_prefix('{')?;
        let close_b = balanced_brace_close(inner)?;
        let body = js_arm_slot(inner[..close_b].trim())?;
        if !inner[close_b + 1..].trim().is_empty() {
            return None;
        }
        JsTail::ElseIf { cond, body }
    } else if let Some(rest) = rest.strip_prefix("else ") {
        let inner = rest.trim_start().strip_prefix('{')?;
        let close_b = balanced_brace_close(inner)?;
        let body = js_arm_slot(inner[..close_b].trim())?;
        if !inner[close_b + 1..].trim().is_empty() {
            return None;
        }
        JsTail::Block(body)
    } else {
        return None;
    };
    Some(JsIfTemplate { cond, body, tail })
}

/// A js arm slot: one canonical meta, `$$`-demanded (the E9 discipline —
/// the served single-`$` faces keep their general-lane route). Condition
/// slots use [`js_cond_slot`] instead: `$A` is the receipted spelling (f1).
pub(crate) fn js_arm_slot(token: &str) -> Option<LaneMeta> {
    if !token.starts_with("$$") || token.starts_with("$$$") {
        return None;
    }
    let meta = lane_meta(token)?;
    if meta.multi {
        return None;
    }
    Some(meta)
}

/// A js condition slot: one non-multi canonical meta.
pub(crate) fn js_cond_slot(token: &str) -> Option<LaneMeta> {
    let meta = lane_meta(token)?;
    if meta.multi {
        return None;
    }
    Some(meta)
}

/// The `async def $N($$P):\n    $$B` slot parse — the `$$` UNIVERSAL
/// spellings are demanded (the single-`$` face keeps its existing answering
/// route). P is param-exact, B the whole body block.
pub(crate) struct PyAsyncDefTemplate {
    name: LaneName,
    param: LaneMeta,
    body: LaneMeta,
}

pub(crate) fn py_async_def_template(pattern: &str) -> Option<PyAsyncDefTemplate> {
    let rest = root142_trivia(pattern).strip_prefix("async def ")?;
    let (name_tok, rest) = root142_split_token(rest)?;
    let params_inner = rest.trim_start().strip_prefix('(')?;
    let close = balanced_paren_close(params_inner)?;
    let param_tok = params_inner[..close].trim();
    // Demand the `$$` universal spelling.
    if !param_tok.starts_with("$$") || param_tok.starts_with("$$$") {
        return None;
    }
    let param = root142_meta_slot(param_tok)?;
    let after = params_inner[close + 1..].trim();
    let body_tok = after.strip_prefix(':')?.trim();
    if !body_tok.starts_with("$$") || body_tok.starts_with("$$$") {
        return None;
    }
    Some(PyAsyncDefTemplate {
        name: root142_name_slot(name_tok)?,
        param,
        body: root142_meta_slot(body_tok)?,
    })
}

/// Split the leading identifier/meta token off `text`; the cut is the
/// FIRST structural delimiter (`ws { ( ; :`) — `async def $N(...)` cuts at
/// the `(`, `declare const y: number;` at the `:`, `mod $N {` at the ws.
pub(crate) fn root142_split_token(text: &str) -> Option<(&str, &str)> {
    let text = root142_trivia(text);
    let end = text
        .find([' ', '\t', '\n', '\r', '{', '(', ';', ':'])
        .unwrap_or(text.len());
    let token = &text[..end];
    if token.is_empty() {
        return None;
    }
    Some((token, &text[end..]))
}

pub(crate) fn named_non_trivia_children<'tree>(node: &Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect()
}

pub(crate) fn has_child_kind<'tree>(node: &Node<'tree>, kind: &str) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|c| c.kind() == kind);
    found
}

pub(crate) fn match_statement_root_142(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    let template = statement_root_142_template(lang, pattern)?;
    let tree = parse_source(lang, source).ok()?;
    let mut out = Vec::new();
    walk_root_142(tree.root_node(), source, pattern, &template, &mut out);
    Some(out)
}

pub(crate) fn walk_root_142(
    node: Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
    out: &mut Vec<PatternMatch>,
) {
    if node.is_named() && !is_in_comment_or_string(&node) {
        let hit = match template {
            Root142Template::RsMod { .. } if node.kind() == "mod_item" => {
                root142_block_member_match(&node, source, pattern, "name", "body", template)
            }
            Root142Template::RsExternCrate { .. } if node.kind() == "extern_crate_declaration" => {
                root142_field_match(&node, source, pattern, "name", None, template)
            }
            Root142Template::GoType { .. } if node.kind() == "type_spec" => {
                // The reference refuses a comment or gap-junk run in the
                // `type`-keyword→name position while the name-internal,
                // pre-tail, and lead/trail comment positions bind. Gate: when
                // the parent declaration leads with the anonymous `type`
                // token, the gap bytes between that token and the spec must
                // be ASCII whitespace only (comment bytes and junk are
                // outside that class; a grouped `type ( … )` declaration
                // does not lead with the token and keeps its prior route).
                let gap_ascii_clean = node.parent().map_or(true, |decl| {
                    let mut kw = decl.walk();
                    decl.children(&mut kw)
                        .next()
                        .is_some_and(|first| !first.is_named() && first.kind() == "type")
                        && source[decl.start_byte() + "type".len()..node.start_byte()]
                            .bytes()
                            .all(|b| b.is_ascii_whitespace())
                });
                if gap_ascii_clean {
                    root142_field_match(&node, source, pattern, "name", Some("type"), template)
                } else {
                    None
                }
            }
            Root142Template::RbModule { .. } if node.kind() == "module" => {
                root142_rb_module_match(&node, source, pattern, template)
            }
            Root142Template::TsDeclareModule { .. } if node.kind() == "ambient_declaration" => {
                root142_ts_module_match(&node, source, pattern, template)
            }
            Root142Template::TsDeclareConst { .. } if node.kind() == "ambient_declaration" => {
                root142_ts_const_match(&node, source, pattern, template)
            }
            Root142Template::PyAsyncDef { .. } if node.kind() == "function_definition" => {
                root142_py_async_match(&node, source, pattern, template)
            }
            Root142Template::JaEnum { .. } if node.kind() == "enum_declaration" => {
                root142_ja_enum_match(&node, source, pattern, template)
            }
            Root142Template::CsRecord { .. } if node.kind() == "record_declaration" => {
                root142_cs_record_match(&node, source, pattern, template)
            }
            Root142Template::CsStruct { .. } if node.kind() == "struct_declaration" => {
                root142_cs_struct_match(&node, source, pattern, template)
            }
            // The statement-root siblings — each walk is kind-exact and binds
            // its slots exactly.
            Root142Template::CsGoto { label } if node.kind() == "goto_statement" => {
                root146_goto_match(&node, source, pattern, label)
            }
            Root142Template::CsNew { ty, arg } if node.kind() == "expression_statement" => {
                root146_cs_new_match(&node, source, pattern, ty, arg)
            }
            Root142Template::CsSwitch { subject, body } if node.kind() == "switch_statement" => {
                root146_cs_switch_match(&node, source, pattern, subject, body)
            }
            Root142Template::PhpDecl { kind, name, body } if node.kind() == kind.root_kind() => {
                root146_php_decl_match(&node, source, pattern, *kind, name, body)
            }
            Root142Template::GoSwitch { subject, body }
                if node.kind() == "expression_switch_statement" =>
            {
                root146_go_switch_match(&node, source, pattern, subject, body)
            }
            Root142Template::GoIf { cond, body } if node.kind() == "if_statement" => {
                root146_go_if_match(&node, source, pattern, cond, body)
            }
            Root142Template::GoPackage { name } if node.kind() == "package_clause" => {
                root146_go_package_match(&node, source, pattern, name)
            }
            Root142Template::GoImport { target } if node.kind() == "import_declaration" => {
                root146_go_import_match(&node, source, pattern, target)
            }
            Root142Template::JsElseIf(tpl) if node.kind() == "if_statement" => {
                root146_js_if_match(&node, source, pattern, tpl)
            }
            _ => None,
        };
        if let Some(hit) = hit {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_root_142(child, source, pattern, template, out);
    }
}

pub(crate) fn root146_named_children<'tree>(node: &Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named() && !is_trivia_kind(child.kind()) && !child.is_extra())
        .collect()
}

/// cs `goto_statement` — the label is the first named child; the candidate
/// gap (comment/U+2028 junk) is transparent (the parse recovers the junk
/// outside the label).
pub(crate) fn root146_goto_match(
    node: &Node,
    source: &str,
    pattern: &str,
    label: &LaneMeta,
) -> Option<PatternMatch> {
    let label_node = root146_named_children(node).first().copied()?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(&label_node, source)?;
    label.bind(&mut captures, text.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// cs `new <T>(<A>)[;]` at the expression_statement root — exactly ONE
/// argument; the nested face refuses naturally (no object-creation child).
pub(crate) fn root146_cs_new_match(
    node: &Node,
    source: &str,
    pattern: &str,
    ty: &LaneName,
    arg: &LaneMeta,
) -> Option<PatternMatch> {
    let children = root146_named_children(node);
    let [creation] = children.as_slice() else {
        return None;
    };
    if creation.kind() != "object_creation_expression" {
        return None;
    }
    let creation_children = root146_named_children(creation);
    let (type_node, arg_list) = match creation_children.as_slice() {
        [type_node, arg_list] if arg_list.kind() == "argument_list" => (type_node, arg_list),
        _ => return None,
    };
    let args = root146_named_children(arg_list);
    let [only_arg] = args.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let type_text = node_text(type_node, source)?;
    ty.bind_or_match(&mut captures, type_text.trim())?;
    let arg_text = node_text(only_arg, source)?;
    arg.bind(&mut captures, arg_text.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// cs switch_statement — the subject slot binds between the parens and the
/// body must carry EXACTLY ONE switch_section (empty/multi refuse).
/// Kind-exact: the switch_expression row never crosses.
pub(crate) fn root146_cs_switch_match(
    node: &Node,
    source: &str,
    pattern: &str,
    subject: &Option<LaneName>,
    body: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let body_node = named.last()?;
    if body_node.kind() != "switch_body" {
        return None;
    }
    let subject_node = if named.len() == 2 {
        Some(&named[0])
    } else {
        None
    };
    match (subject, subject_node) {
        (None, Some(_)) | (Some(_), None) => return None,
        _ => {}
    }
    let sections = root146_named_children(body_node);
    let [section] = sections.as_slice() else {
        return None;
    };
    if section.kind() != "switch_section" {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    if let (Some(slot), Some(subject_node)) = (subject, subject_node) {
        let text = node_text(subject_node, source)?;
        slot.bind_or_match(&mut captures, text.trim())?;
    }
    let text = node_text(section, source)?;
    body.bind(&mut captures, text)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// php declaration kinds — the ONE-member meta law and the Mixed
/// exact-order face, modeled on the registered namespace lane; the Mixed-doc
/// wrapper uses the SAME declaration kind so member-kind alignment holds
/// (class property/const prefixes).
pub(crate) fn root146_php_decl_match(
    node: &Node,
    source: &str,
    pattern: &str,
    kind: PhpDeclKind,
    name: &LaneName,
    body: &PhpNamespaceBody,
) -> Option<PatternMatch> {
    let name_node = node.child_by_field_name("name")?;
    let body_node = node.child_by_field_name("body")?;
    let members = root146_named_children(&body_node);
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_text = node_text(&name_node, source)?;
    name.bind_or_match(&mut captures, name_text.trim())?;
    match body {
        PhpNamespaceBody::Meta(body_meta) => {
            let [only] = members.as_slice() else {
                return None;
            };
            let text = node_text(only, source)?;
            body_meta.bind(&mut captures, text)?;
        }
        PhpNamespaceBody::Mixed { prefix, tail } => {
            let doc = format!("<?php\n{kw} __Px {{ {prefix} }}\n", kw = kind.keyword());
            let tpl_tree = parse_source(Language::Php, &doc).ok()?;
            if tpl_tree.root_node().has_error() {
                return None;
            }
            let prefix_members: Vec<Node> = tpl_tree
                .root_node()
                .children(&mut tpl_tree.root_node().walk())
                .find(|n| n.kind() == kind.root_kind())
                .and_then(|decl| decl.child_by_field_name("body"))
                .map(|body| root146_named_children(&body))
                .unwrap_or_default();
            if prefix_members.is_empty() || members.len() != prefix_members.len() + 1 {
                return None;
            }
            let normalize = |node: &Node, src: &str| -> Option<String> {
                Some(
                    strip_comment_spans(node_text(node, src)?)
                        .split_whitespace()
                        .collect(),
                )
            };
            for (p_stmt, c_stmt) in prefix_members.iter().zip(members.iter()) {
                if normalize(p_stmt, &doc) != normalize(c_stmt, source) {
                    return None;
                }
            }
            let tail_text = node_text(&members[members.len() - 1], source)?.trim();
            tail.bind(&mut captures, tail_text)?;
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}

/// go expression_switch_statement — optional subject slot (demarcated: a
/// global pattern refuses a subject-bearing candidate and vice versa) and
/// EXACTLY ONE case clause (2-clause/empty refuse). The clause text binds
/// RAW (the capture keeps the trailing newline). Type-switch candidates are
/// a distinct kind.
pub(crate) fn root146_go_switch_match(
    node: &Node,
    source: &str,
    pattern: &str,
    subject: &Option<LaneName>,
    body: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let is_clause = |n: &Node| n.kind().ends_with("_case") || n.kind() == "default_case";
    // tree-sitter-go wraps the clauses in a `switch_body` child; the
    // subject-less face must not mistake that wrapper for the subject.
    let body_wrapper = named.iter().find(|n| n.kind() == "switch_body").copied();
    let clauses: Vec<Node> = match &body_wrapper {
        Some(wrapper) => root146_named_children(wrapper)
            .into_iter()
            .filter(|n| is_clause(n))
            .collect(),
        None => named.iter().filter(|n| is_clause(n)).copied().collect(),
    };
    let subject_node = named
        .iter()
        .find(|n| !is_clause(n) && n.kind() != "switch_body");
    match (subject, subject_node) {
        (None, Some(_)) | (Some(_), None) => return None,
        _ => {}
    }
    let [clause] = clauses.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    if let (Some(slot), Some(subject_node)) = (subject, subject_node) {
        let text = node_text(subject_node, source)?;
        slot.bind_or_match(&mut captures, text.trim())?;
    }
    let text = node_text(clause, source)?;
    body.bind(&mut captures, text)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// go if_statement — C binds the raw condition bytes between the `if`
/// keyword and the body; B binds the WHOLE body inner text and refuses the
/// empty body.
pub(crate) fn root146_go_if_match(
    node: &Node,
    source: &str,
    pattern: &str,
    cond: &LaneName,
    body: &LaneMeta,
) -> Option<PatternMatch> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let if_kw = children.first()?;
    if if_kw.kind() != "if" {
        return None;
    }
    let block = children
        .iter()
        .find(|child| child.is_named() && child.kind() == "block")?;
    let cond_node = children.iter().find(|child| child.is_named())?;
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let cond_text = source[if_kw.end_byte()..block.start_byte()].trim();
    if cond_text.is_empty() {
        return None;
    }
    let _ = cond_node;
    cond.bind_or_match(&mut captures, cond_text)?;
    let inner = &source[block.start_byte() + 1..block.end_byte().saturating_sub(1)];
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    body.bind(&mut captures, inner)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// go package_clause — the name binds through a comment gap (the comment
/// is an extra outside the identifier).
pub(crate) fn root146_go_package_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let [name_node] = named.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(name_node, source)?;
    name.bind(&mut captures, text.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// go SEMI-LESS import_declaration — the single path or the WHOLE group
/// text binds (X raw).
pub(crate) fn root146_go_import_match(
    node: &Node,
    source: &str,
    pattern: &str,
    target: &LaneMeta,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let [spec] = named.as_slice() else {
        return None;
    };
    if spec.kind() != "import_spec" && spec.kind() != "import_spec_list" {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let text = node_text(spec, source)?;
    target.bind(&mut captures, text)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// js if_statement arms — ONE-statement bodies; the else-if tail binds the
/// INNER if's head only and the walk's own recursion emits per level; the
/// plain-else and no-else tails demarcate.
pub(crate) fn root146_js_if_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &JsIfTemplate,
) -> Option<PatternMatch> {
    let named = root146_named_children(node);
    let [cond_node, body_node, tail_node] = named.as_slice() else {
        let [cond_node, body_node] = named.as_slice() else {
            return None;
        };
        return root146_js_if_bind(node, source, pattern, template, cond_node, body_node, None);
    };
    root146_js_if_bind(
        node,
        source,
        pattern,
        template,
        cond_node,
        body_node,
        Some(tail_node),
    )
}

pub(crate) fn root146_js_if_bind(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &JsIfTemplate,
    cond_node: &Node,
    body_node: &Node,
    tail_node: Option<&Node>,
) -> Option<PatternMatch> {
    if cond_node.kind() != "parenthesized_expression" || body_node.kind() != "statement_block" {
        return None;
    }
    let cond_inner = root146_named_children(cond_node);
    let [cond_expr] = cond_inner.as_slice() else {
        return None;
    };
    let body_stmts = root146_named_children(body_node);
    let [body_stmt] = body_stmts.as_slice() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let cond_text = node_text(cond_expr, source)?;
    template.cond.bind(&mut captures, cond_text.trim())?;
    let body_text = node_text(body_stmt, source)?;
    template.body.bind(&mut captures, body_text)?;
    match &template.tail {
        JsTail::None => {
            if tail_node.is_some() {
                return None;
            }
        }
        JsTail::Block(meta) => {
            let tail_node = tail_node?;
            if tail_node.kind() != "else_clause" {
                return None;
            }
            let else_body = root146_named_children(tail_node);
            let [else_block] = else_body.as_slice() else {
                return None;
            };
            if else_block.kind() != "statement_block" {
                return None;
            }
            let stmts = root146_named_children(else_block);
            let [stmt] = stmts.as_slice() else {
                return None;
            };
            let text = node_text(stmt, source)?;
            meta.bind(&mut captures, text)?;
        }
        JsTail::ElseIf { cond, body } => {
            let tail_node = tail_node?;
            if tail_node.kind() != "else_clause" {
                return None;
            }
            let inner = root146_named_children(tail_node);
            let [inner_if] = inner.as_slice() else {
                return None;
            };
            if inner_if.kind() != "if_statement" {
                return None;
            }
            let inner_named = root146_named_children(inner_if);
            // The inner if may itself carry a FURTHER else-if tail: only
            // cond+body feed this level's C/D binds; the walk emits the
            // deeper levels at their own if nodes.
            if inner_named.len() < 2 || inner_named.len() > 3 {
                return None;
            }
            let (inner_cond, inner_body) = (&inner_named[0], &inner_named[1]);
            if inner_cond.kind() != "parenthesized_expression"
                || inner_body.kind() != "statement_block"
            {
                return None;
            }
            let inner_cond_expr = root146_named_children(inner_cond);
            let [inner_cond_expr] = inner_cond_expr.as_slice() else {
                return None;
            };
            let inner_stmts = root146_named_children(inner_body);
            let [inner_stmt] = inner_stmts.as_slice() else {
                return None;
            };
            let cond_text = node_text(inner_cond_expr, source)?;
            cond.bind(&mut captures, cond_text.trim())?;
            let body_text = node_text(inner_stmt, source)?;
            body.bind(&mut captures, body_text)?;
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}

/// Shared slot binding: a name field (LaneName) and an optional second
/// field (LaneName against the field node's text).
pub(crate) fn root142_field_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name_field: &str,
    second_field: Option<&str>,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let (name, second) = match template {
        Root142Template::RsExternCrate { name } => (name, None),
        Root142Template::GoType { name, ty } => (name, Some(ty)),
        _ => return None,
    };
    let _ = second_field;
    // Modifier doctrine: a visibility/modifier child the pattern lacks
    // breaks the reference child alignment.
    if has_child_kind(node, "visibility_modifier") || has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name(name_field)?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    if let Some(ty) = second {
        let ty_node = node.child_by_field_name("type")?;
        ty.bind_or_match(&mut captures, node_text(&ty_node, source)?.trim())?;
    }
    Some(hit_for_node(node, source, pattern, captures))
}

/// `module <Name>\n  $B\nend` — the name field binds N; the body field
/// (the `body_statement`) binds B as the WHOLE trimmed middle (E3/E3b).
pub(crate) fn root142_rb_module_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::RbModule { name, body } = template else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = node.child_by_field_name("body")?;
    body.bind(&mut captures, node_text(&body_node, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// The `{ $B }` block law: the body field's named non-trivia children are
/// SINGLE-EXACT — exactly one member binds its text (E2/K1/K3); 0/≥2
/// refuse. The modifier doctrine applies to the enclosing node.
pub(crate) fn root142_block_member_match(
    node: &Node,
    source: &str,
    pattern: &str,
    name_field: &str,
    body_field: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::RsMod { name, body } = template else {
        return None;
    };
    if has_child_kind(node, "visibility_modifier") || has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name(name_field)?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = node.child_by_field_name(body_field)?;
    let members = named_non_trivia_children(&body_node);
    let [only] = members.as_slice() else {
        return None;
    };
    body.bind(&mut captures, node_text(only, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// `declare module <name> { $B }` — the ambient_declaration wraps a module
/// node (name field carries the QUOTES, E10); the statement block is
/// single-statement-exact (E10b).
pub(crate) fn root142_ts_module_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::TsDeclareModule { name, body } = template else {
        return None;
    };
    let module_node = node.named_child(0)?;
    // `declare module "x"` / `declare module x` / `namespace` — the
    // ambient child is the ts `module` or `internal_module` node (both
    // carry name!/body? fields).
    if module_node.kind() != "module" && module_node.kind() != "internal_module" {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = module_node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = module_node.child_by_field_name("body")?;
    let members = named_non_trivia_children(&body_node);
    let [only] = members.as_slice() else {
        return None;
    };
    body.bind(&mut captures, node_text(only, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// `declare const <X>: <T>;` — the lexical_declaration's kind token must be
/// `const` (H23); X = the declarator name, T = the annotation text minus
/// the `:` (E11: T='number').
pub(crate) fn root142_ts_const_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::TsDeclareConst { name, ty } = template else {
        return None;
    };
    let decl = node.named_child(0)?;
    if decl.kind() != "lexical_declaration" {
        return None;
    }
    let mut cursor = decl.walk();
    let kind_tok = decl
        .child_by_field_name("kind")
        .or_else(|| decl.children(&mut cursor).find(|c| !c.is_named()))?;
    if node_text(&kind_tok, source)?.trim() != "const" {
        return None;
    }
    let declarators = named_non_trivia_children(&decl);
    let Some(declarator) = declarators.first() else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = declarator.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let ty_node = declarator.child_by_field_name("type")?;
    let ty_text = node_text(&ty_node, source)?.trim();
    let ty_text = ty_text.strip_prefix(':').unwrap_or(ty_text).trim();
    ty.bind_or_match(&mut captures, ty_text)?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// `async def <N>($$P):\n    $$B` — the function_definition must carry the
/// `async` token (sync faces keep their existing routes); P is
/// PARAM-EXACT; B = the whole body block text trimmed (E8b).
pub(crate) fn root142_py_async_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::PyAsyncDef { name, param, body } = template else {
        return None;
    };
    let mut cursor = node.walk();
    let is_async = node
        .children(&mut cursor)
        .any(|c| !c.is_named() && node_text(&c, source).is_some_and(|t| t == "async"));
    if !is_async {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let params_node = node.child_by_field_name("parameters")?;
    let params = named_non_trivia_children(&params_node);
    let [only] = params.as_slice() else {
        return None;
    };
    param.bind(&mut captures, node_text(only, source)?.trim())?;
    let body_node = node.child_by_field_name("body")?;
    body.bind(&mut captures, node_text(&body_node, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// `enum <name> { <body> }` — members are the enum_constant children plus
/// the declarations inside `enum_body_declarations`; the body slot is
/// SINGLE-EXACT (K3/G3), the literal slot byte-matches (H3), and the EMPTY
/// slot binds 0-member candidates (K4). Modifier candidates refuse (G3).
pub(crate) fn root142_ja_enum_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::JaEnum { name, body } = template else {
        return None;
    };
    if has_child_kind(node, "modifiers") {
        return None;
    }
    let body_node = node.child_by_field_name("body")?;
    let mut members: Vec<Node> = named_non_trivia_children(&body_node)
        .into_iter()
        .flat_map(|child| {
            if child.kind() == "enum_body_declarations" {
                named_non_trivia_children(&child)
            } else {
                vec![child]
            }
        })
        .collect();
    if matches!(body, JaEnumBody::Meta(_)) {
        // The `{`/`}` around an EMPTY member list can surface as an
        // anonymous-token-only body; nothing to filter — 0 members is 0.
        members.retain(|m| m.kind() != "enum_body_declarations");
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    match body {
        JaEnumBody::Empty => {
            if !members.is_empty() {
                return None;
            }
        }
        JaEnumBody::Meta(meta) => {
            let [only] = members.as_slice() else {
                return None;
            };
            meta.bind(&mut captures, node_text(only, source)?.trim())?;
        }
        JaEnumBody::Literal(literal) => {
            let [only] = members.as_slice() else {
                return None;
            };
            if node_text(only, source)?.trim() != literal {
                return None;
            }
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}

/// `record <name>(<P>);[ { $B }]` — the parameter slot is PARAM-EXACT
/// (K2/K7), the optional body single-member-exact (J6); modifier
/// candidates refuse (G4).
pub(crate) fn root142_cs_record_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::CsRecord { name, param, body } = template else {
        return None;
    };
    if has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    // NOTE: the cs grammar gives record_declaration NO parameter_list
    // FIELD (only name/body are fields) — the list is a fieldless child.
    let params_node = named_non_trivia_children(node)
        .into_iter()
        .find(|c| c.kind() == "parameter_list")?;
    let params: Vec<Node> = named_non_trivia_children(&params_node)
        .into_iter()
        .filter(|c| c.kind() == "parameter")
        .collect();
    let [only] = params.as_slice() else {
        return None;
    };
    param.bind(&mut captures, node_text(only, source)?.trim())?;
    let body_node = node.child_by_field_name("body");
    match (body, body_node) {
        (None, None) => {}
        (None, Some(_)) => return None,
        (Some(_), None) => return None,
        (Some(slot), Some(body_node)) => {
            let members = named_non_trivia_children(&body_node);
            let [only] = members.as_slice() else {
                return None;
            };
            slot.bind(&mut captures, node_text(only, source)?.trim())?;
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}

/// `struct <name> { $B }` — the body is SINGLE- MEMBER-EXACT (K1); empty
/// bodies refuse (J9); modifier candidates refuse (G6).
pub(crate) fn root142_cs_struct_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &Root142Template,
) -> Option<PatternMatch> {
    let Root142Template::CsStruct { name, body } = template else {
        return None;
    };
    if has_child_kind(node, "modifier") {
        return None;
    }
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    name.bind_or_match(&mut captures, node_text(&name_node, source)?.trim())?;
    let body_node = node.child_by_field_name("body")?;
    let members = named_non_trivia_children(&body_node);
    let [only] = members.as_slice() else {
        return None;
    };
    body.bind(&mut captures, node_text(only, source)?.trim())?;
    Some(hit_for_node(node, source, pattern, captures))
}

/// Directive-family PATTERN spellings the reference ACCEPTS but binds
/// NOTHING on (census answerable, the walk's empty is the agreement,
/// never loud):
///   * a comment span (or, in cs, a Rust-whitespace char outside the cs
///     trivia class) inside the demarcation zone of a family pattern;
///   * a plain-face cs using pattern whose target is a dotted/qualified
///     META path (`$A.B`, `static $N.M`, `global::$N`).
pub(crate) fn directive_pattern_sg_accepts_empty(lang: Language, pattern: &str) -> bool {
    // The META-face class only: a `$`-less (literal) comment face keeps its
    // own answering route (the reference binds comment-transparent literals).
    if !pattern.contains('$') {
        return false;
    }
    if !matches!(lang, Language::CSharp | Language::Php | Language::Java) {
        return false;
    }
    let Some(semi_at) = pattern.rfind(';') else {
        return false;
    };
    // A non-empty tail after the last `;` is a different face (comment
    // tails keep their own census class).
    if !pattern[semi_at + 1..].trim().is_empty() {
        return false;
    }
    let zone = &pattern[..=semi_at];
    let stripped = strip_comment_spans(zone);
    let fixed = if lang == Language::CSharp {
        stripped
            .chars()
            .map(|c| {
                if !is_sg_cs_trivia(c) && c.is_whitespace() {
                    ' '
                } else {
                    c
                }
            })
            .collect::<String>()
    } else {
        stripped
    };
    fixed != zone && directive_template(&fixed).is_some()
}

/// The plain-face cs using dotted/qualified-META path family — every
/// segment a canonical meta or an identifier, >= 2 segments, >= 1 meta
/// (the reference binds NOTHING on any candidate).
pub(crate) fn cs_using_plainface_meta_path(pattern: &str) -> bool {
    let p = pattern.trim();
    let mut rest = p;
    if let Some(after) = rest.strip_prefix("global") {
        if after.starts_with(is_sg_cs_trivia) {
            rest = after.trim_start_matches(is_sg_cs_trivia);
        }
    }
    let Some(after) = rest.strip_prefix("using") else {
        return false;
    };
    if !after.starts_with(is_sg_cs_trivia) {
        return false;
    }
    let mut rest = after.trim_start_matches(is_sg_cs_trivia);
    loop {
        let mut consumed = false;
        for keyword in [CsUsingKeyword::Static, CsUsingKeyword::Unsafe] {
            if let Some(seg) = rest.strip_prefix(keyword.spell()) {
                if seg.starts_with(is_sg_cs_trivia) {
                    rest = seg.trim_start_matches(is_sg_cs_trivia);
                    consumed = true;
                    break;
                }
            }
        }
        if !consumed {
            break;
        }
    }
    let Some(inner) = rest.strip_suffix(';') else {
        return false;
    };
    let inner = inner.trim_matches(is_sg_cs_trivia);
    if inner.is_empty() || top_level_equals(inner).is_some() {
        return false;
    }
    let segments: Vec<&str> = inner.split("::").flat_map(|s| s.split('.')).collect();
    segments.len() >= 2
        && segments.iter().any(|s| lane_meta(s).is_some())
        && segments
            .iter()
            .all(|s| lane_meta(s).is_some() || is_pattern_ident(s))
}

/// The language-free ingress unions — these faces must reach the
/// per-language census, not the query-level structural fallback.
pub(crate) fn statement_root_142_any_language(pattern: &str) -> bool {
    [
        Language::Rust,
        Language::Go,
        Language::Ruby,
        Language::TypeScript,
        Language::Python,
        Language::Java,
        Language::CSharp,
        // The php declaration-kind meta bodies and the js `$$`-arm faces
        // join the language-free union.
        Language::JavaScript,
        Language::Php,
    ]
    .iter()
    .any(|&lang| statement_root_142_template(lang, pattern).is_some())
}

pub(crate) fn directive_accepted_empty_any_language(pattern: &str) -> bool {
    [Language::CSharp, Language::Php, Language::Java]
        .iter()
        .any(|&lang| {
            directive_pattern_sg_accepts_empty(lang, pattern)
                || (lang == Language::CSharp && cs_using_plainface_meta_path(pattern))
        })
}
