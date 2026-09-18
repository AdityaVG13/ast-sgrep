//! Import/directive/using root lanes.

use super::*;
use crate::extract::{is_in_comment_or_string, node_text};
use crate::Language;
use std::collections::BTreeMap;
use tree_sitter::Node;

// ===========================================================================
// Directive/import roots, the csharp checked/unchecked EXPRESSION root, java
// synchronized nested-block + METHOD faces, and the remaining statement roots
// (kotlin typealias/for, swift for, rust let-else, c goto).
//
// Binding laws encoded here:
//   * py `import $X` binds each import_statement; an aliased child binds the
//     whole child text; `import $X as $Y` / `from $M import $X` split slots.
//   * java `import [static] T[.*];`: static/star decorations are DISTINCT
//     faces — a plain pattern refuses static/star candidates.
//   * rs/php `use $X;` binds the whole clause; `use $X as $Y;` splits at the
//     top-level `as`. cs `using $N;` binds the namespace, refuses `=`
//     aliases and the using-STATEMENT face.
//   * go `goto $L;` / `import $X;` are walk-empty bind-nothing spellings;
//     c `goto $L;` BINDS every goto label.
//   * kotlin `typealias $N = $T` binds both slots; kotlin `for` binds X/C
//     and the block-inner text trimmed both ends, brace-less bodies refuse.
//   * swift `for ... [where $W]`: `where` presence must agree on both
//     sides; B keeps trailing whitespace.
//   * rs `let $P = $E else { $B };` binds P/E and the ONE-statement else
//     body; no-else candidates refuse.
//   * csharp `checked($E)`/`unchecked($E)` are EXPRESSION roots binding the
//     inner text; statement-position candidates refuse.
//   * java synchronized compositions bind every slot at its own level; the
//     METHOD face hops ordinary keyword modifiers positionally while pattern
//     modifiers demand ordered carriage; one-statement body law every slot.
// ===========================================================================

/// A lane metavariable slot with its namespace: `$$$NAME` binds the MULTI
/// namespace (key `$$$NAME` in the capture map), `$`/`$$NAME` the single
/// one (java `synchronized ($$$X)` binds multi X, `synchronized ($$X)`
/// binds single X, py `del d[$$$K]` binds multi K).
#[derive(Debug, Clone)]
pub(crate) struct LaneMeta {
    pub(crate) name: String,
    pub(crate) multi: bool,
}

pub(crate) fn lane_meta(token: &str) -> Option<LaneMeta> {
    let token = token.trim();
    if let Some(name) = token.strip_prefix("$$$") {
        return is_metavar_name(name).then_some(LaneMeta {
            name: name.to_string(),
            multi: true,
        });
    }
    capture_name(token).map(|name| LaneMeta {
        name: name.to_string(),
        multi: false,
    })
}

impl LaneMeta {
    pub(crate) fn bind(&self, captures: &mut BTreeMap<String, String>, text: &str) -> Option<()> {
        bind_capture_kind(captures, &self.name, text, self.multi)
    }
}

/// A name slot that is either a lane meta or a byte-matched literal
/// (`using System;` — the reference answers the directive node where the
/// cs statement lane over-served).
#[derive(Debug, Clone)]
pub(crate) enum LaneName {
    Meta(LaneMeta),
    Literal(String),
}

impl LaneName {
    pub(crate) fn bind_or_match(
        &self,
        captures: &mut BTreeMap<String, String>,
        text: &str,
    ) -> Option<()> {
        match self {
            LaneName::Meta(meta) => meta.bind(captures, text),
            LaneName::Literal(literal) => (text == literal).then_some(()),
        }
    }
}

/// The cs using-alias rhs slot faces — the structural law beyond the
/// single-meta spelling: `($T, $U)` is the TUPLE face (element slots,
/// count-exact), `$T[]` is the ARRAY-SUFFIX face (T binds the element type;
/// the suffix strips recursively), and `global::$T` is the QUALIFIED face (T
/// binds the text AFTER the `global::` qualifier).
#[derive(Debug, Clone)]
pub(crate) enum AliasRhs {
    One(LaneName),
    Tuple(Vec<LaneName>),
    ArraySuffix(LaneName),
    QualifiedGlobalMeta(LaneMeta),
}

impl AliasRhs {
    /// Bind the alias rhs against the candidate's rhs text (already
    /// comment-stripped and trimmed by the caller).
    pub(crate) fn bind_or_match(
        &self,
        captures: &mut BTreeMap<String, String>,
        text: &str,
    ) -> Option<()> {
        match self {
            AliasRhs::One(name) => name.bind_or_match(captures, text),
            AliasRhs::Tuple(slots) => {
                // Count-exact element alignment over the depth-aware comma
                // split (a 3-element candidate refuses a 2-slot pattern).
                let inner = text.strip_prefix('(')?.strip_suffix(')')?;
                let elements = split_top_level_commas(inner);
                if elements.len() != slots.len() {
                    return None;
                }
                for (slot, element) in slots.iter().zip(elements.iter()) {
                    slot.bind_or_match(captures, element.trim())?;
                }
                Some(())
            }
            AliasRhs::ArraySuffix(name) => {
                // The candidate must carry the suffix; the element binds the
                // trimmed prefix (H4 recursion: `int[][]` → `int[]`).
                let element = text.strip_suffix(']')?.strip_suffix('[')?;
                name.bind_or_match(captures, element.trim())
            }
            AliasRhs::QualifiedGlobalMeta(meta) => {
                let rest = text.strip_prefix("global::")?;
                if rest.is_empty() || rest.contains(char::is_whitespace) {
                    return None;
                }
                meta.bind(captures, rest)
            }
        }
    }
}

/// Parse the ALIAS rhs slot faces. `text` is the trimmed rhs of the `=`
/// split. Falls back to the single-slot face only when no tuple/array/
/// qualified shape applies.
pub(crate) fn alias_rhs_slot(text: &str) -> Option<AliasRhs> {
    if let Some(element) = text.strip_suffix("[]").map(str::trim) {
        if let Some(slot) = lane_slot(element) {
            return Some(AliasRhs::ArraySuffix(slot));
        }
    }
    if text.starts_with('(') && text.ends_with(')') {
        let inner = &text[1..text.len() - 1];
        let slots = split_top_level_commas(inner)
            .into_iter()
            .map(|element| lane_slot(element.trim()))
            .collect::<Option<Vec<_>>>()?;
        if !slots.is_empty() {
            return Some(AliasRhs::Tuple(slots));
        }
    }
    if let Some(rest) = text.strip_prefix("global::") {
        if let Some(meta) = lane_meta(rest) {
            return Some(AliasRhs::QualifiedGlobalMeta(meta));
        }
    }
    lane_slot(text).map(AliasRhs::One)
}

/// A lane name slot: canonical meta or dotted ident path literal.
pub(crate) fn lane_slot(text: &str) -> Option<LaneName> {
    if let Some(meta) = lane_meta(text) {
        return Some(LaneName::Meta(meta));
    }
    directive_path_literal(text).then(|| LaneName::Literal(text.to_string()))
}

/// A dotted/literal path text the directive lanes byte-match
/// (`java.util.List`, `System.Text`). `::`-qualified C# namespace-alias paths
/// (`global::System.Math`, alias-qualified `System::Math`) ride the same
/// ident-segment law — the reference binds the whole qualified text.
pub(crate) fn directive_path_literal(text: &str) -> bool {
    let normalized = text.replace("::", ".");
    let mut segments = normalized.split('.');
    let first_ok = segments.next().is_some_and(|s| {
        !s.is_empty()
            && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    });
    first_ok
        && segments.all(|s| {
            !s.is_empty()
                && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// Split at a top-level ` keyword ` occurrence (bracket-depth aware) —
/// the `as`-alias splits (`import $X as $Y`, `use $X as $Y;`).
pub(crate) fn split_top_level_keyword<'a>(
    text: &'a str,
    keyword: &str,
) -> Option<(&'a str, &'a str)> {
    let mut depth = 0usize;
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b' ' if depth == 0 && text[i..].starts_with(keyword) => {
                return Some((&text[..i], &text[i + keyword.len()..]));
            }
            _ => {}
        }
    }
    None
}

// --- E (140A-F5): the import/directive root family. ---

#[derive(Debug, Clone)]
pub(crate) enum DirectiveTemplate {
    /// py `import …` (NO `;`): plain names or the alias face.
    PyImport {
        names: Vec<LaneMeta>,
        alias: Option<(LaneMeta, LaneMeta)>,
    },
    /// py `from $M import …` — comma-separated name slots (meta or literal),
    /// the `*` star face, and the parenthesized-list face (which DEMARCATES:
    /// a paren face refuses a paren-free candidate and vice versa).
    PyFromImport {
        module: LaneMeta,
        names: Vec<LaneName>,
        star: bool,
        paren_wrapped: bool,
    },
    /// java `import [static] T[.*];` / rs|php `use …;`.
    Semi {
        head: &'static str,
        static_prefix: bool,
        star_suffix: bool,
        target: LaneName,
        alias: Option<(LaneMeta, LaneMeta)>,
    },
    /// The cs using-directive family — `using [static|unsafe] T;`,
    /// `using A = T;`, and the `global `-prefixed faces. `global` is
    /// DEMAND-ONLY (a pattern without it binds global candidates too; a
    /// pattern WITH it refuses non-global candidates); the keyword run must
    /// EQUAL the candidate's.
    CsUsing {
        global_prefix: bool,
        keyword_run: Vec<CsUsingKeyword>,
        target: LaneName,
        alias_type: Option<AliasRhs>,
    },
    /// The php `use function|const $X;` kind faces — the literal kind keyword
    /// must agree with the candidate's kind keyword; X binds the name.
    PhpUseKind {
        kind: &'static str,
        target: LaneName,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CsUsingKeyword {
    Static,
    Unsafe,
}

impl CsUsingKeyword {
    pub(crate) fn spell(self) -> &'static str {
        match self {
            CsUsingKeyword::Static => "static",
            CsUsingKeyword::Unsafe => "unsafe",
        }
    }
}

pub(crate) fn directive_template(pattern: &str) -> Option<DirectiveTemplate> {
    let p = pattern.trim();
    if let Some(rest) = p.strip_prefix("from ") {
        // `from $M import …` — the module is a canonical meta; the import
        // list is comma separated name slots (meta or literal), the `*` star
        // face, or the parenthesized spelling of either — the parens
        // DEMARCATE (a paren face refuses a paren-free candidate and vice
        // versa).
        let import_at = rest.find(" import ")?;
        let module = lane_meta(rest[..import_at].trim())?;
        let mut name_section = rest[import_at + " import ".len()..].trim();
        let paren_wrapped = name_section.starts_with('(') && name_section.ends_with(')');
        if paren_wrapped {
            name_section = name_section[1..name_section.len() - 1].trim();
        }
        if name_section == "*" {
            if paren_wrapped {
                return None;
            }
            return Some(DirectiveTemplate::PyFromImport {
                module,
                names: Vec::new(),
                star: true,
                paren_wrapped: false,
            });
        }
        if name_section.is_empty() {
            return None;
        }
        let slot = |element: &str| -> Option<LaneName> {
            let element = element.trim();
            if let Some(meta) = lane_meta(element) {
                return Some(LaneName::Meta(meta));
            }
            is_pattern_ident(element).then(|| LaneName::Literal(element.to_string()))
        };
        let names = split_top_level_commas(name_section)
            .into_iter()
            .map(slot)
            .collect::<Option<Vec<_>>>()?;
        if names.is_empty() {
            return None;
        }
        return Some(DirectiveTemplate::PyFromImport {
            module,
            names,
            star: false,
            paren_wrapped,
        });
    }
    if let Some(rest) = p.strip_prefix("import") {
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let rest = rest.trim_start();
        if rest.contains(';') {
            // java shape: `import [static] T[.*];` — the decorations are
            // part of the FACE.
            let mut rest = rest;
            let mut static_prefix = false;
            if let Some(after) = rest.strip_prefix("static") {
                if after.starts_with(char::is_whitespace) {
                    static_prefix = true;
                    rest = after.trim_start();
                }
            }
            let inner = rest.strip_suffix(';')?;
            let mut star_suffix = false;
            let mut target_text = inner.trim();
            if let Some(prefix) = target_text.strip_suffix(".*") {
                star_suffix = true;
                target_text = prefix.trim_end();
            }
            if target_text.is_empty() || target_text.contains(char::is_whitespace) {
                return None;
            }
            let target = if let Some(meta) = lane_meta(target_text) {
                LaneName::Meta(meta)
            } else if directive_path_literal(target_text) {
                LaneName::Literal(target_text.to_string())
            } else {
                return None;
            };
            return Some(DirectiveTemplate::Semi {
                head: "import",
                static_prefix,
                star_suffix,
                target,
                alias: None,
            });
        }
        // python faces: names / alias; `;`-ful spellings never reach here.
        if let Some((left, right)) = split_top_level_keyword(rest, " as ") {
            let x = lane_meta(left.trim())?;
            let y = lane_meta(right.trim())?;
            return Some(DirectiveTemplate::PyImport {
                names: Vec::new(),
                alias: Some((x, y)),
            });
        }
        let names = split_top_level_commas(rest)
            .into_iter()
            .map(|element| lane_meta(element.trim()))
            .collect::<Option<Vec<_>>>()?;
        // Any slot count — the zip law absorbs trailing candidate names and
        // refuses a short candidate.
        if names.is_empty() {
            return None;
        }
        return Some(DirectiveTemplate::PyImport { names, alias: None });
    }
    if let Some(rest) = p.strip_prefix("use ") {
        let inner = rest.trim().strip_suffix(';')?;
        if inner.is_empty() || inner.contains(',') {
            return None;
        }
        // The php kind faces — a literal `function`/`const` keyword + one
        // canonical name slot. The kind keyword must AGREE with the
        // candidate's; the plain `use $X;` face keeps its whole-clause law.
        for kind in ["function", "const"] {
            if let Some(name) = inner.strip_prefix(kind) {
                if name.starts_with(char::is_whitespace) {
                    let name = name.trim();
                    let target = if let Some(meta) = lane_meta(name) {
                        LaneName::Meta(meta)
                    } else if is_pattern_ident(name) {
                        LaneName::Literal(name.to_string())
                    } else {
                        return None;
                    };
                    return Some(DirectiveTemplate::PhpUseKind { kind, target });
                }
            }
        }
        if let Some((left, right)) = split_top_level_keyword(inner, " as ") {
            let x = lane_meta(left.trim())?;
            let y = lane_meta(right.trim())?;
            return Some(DirectiveTemplate::Semi {
                head: "use",
                static_prefix: false,
                star_suffix: false,
                target: LaneName::Meta(x.clone()),
                alias: Some((x, y)),
            });
        }
        // meta-only: the literal rs/php `use` spellings AGREE on their
        // current literal-lane route (R2_rs_use_lit / R2_php_use_lit).
        let target = lane_meta(inner.trim()).map(LaneName::Meta)?;
        return Some(DirectiveTemplate::Semi {
            head: "use",
            static_prefix: false,
            star_suffix: false,
            target,
            alias: None,
        });
    }
    // The cs using-directive family — `[global ]using [static|unsafe] (T
    // | A = T);`. A `global` prefix is a DEMAND-ONLY marker (a pattern
    // without it binds global candidates too; a pattern with it refuses
    // non-global candidates); the `static`/`unsafe` keyword run is part of
    // the FACE (the run must equal the candidate's); the `= ` alias face
    // splits name/type. Every demarcation skip is CLASS-STRICT
    // (is_sg_cs_trivia) — reference-class trivia runs (U+FEFF/NBSP/VT)
    // parse and BIND while Rust-whitespace outsiders (U+2028 et al) refuse
    // here and fall to the accepted-empty class.
    let mut global_prefix = false;
    let mut using_body = p;
    if let Some(after) = p.strip_prefix("global") {
        if after.starts_with(is_sg_cs_trivia) {
            global_prefix = true;
            using_body = after.trim_start_matches(is_sg_cs_trivia);
        }
    }
    if let Some(rest) = using_body.strip_prefix("using") {
        if !rest.starts_with(is_sg_cs_trivia) {
            return None;
        }
        let mut keyword_run = Vec::new();
        let mut rest = rest.trim_start_matches(is_sg_cs_trivia);
        loop {
            let mut consumed = false;
            for keyword in [CsUsingKeyword::Static, CsUsingKeyword::Unsafe] {
                if let Some(after) = rest.strip_prefix(keyword.spell()) {
                    if after.starts_with(is_sg_cs_trivia) {
                        keyword_run.push(keyword);
                        rest = after.trim_start_matches(is_sg_cs_trivia);
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
            return None;
        };
        // The alias face: split at the TOP-LEVEL `=` (depth-aware over
        // paren/bracket/brace groups).
        if let Some(eq_at) = top_level_equals(inner) {
            let (lhs, rhs) = (
                inner[..eq_at].trim_matches(is_sg_cs_trivia),
                inner[eq_at + 1..].trim_matches(is_sg_cs_trivia),
            );
            if lhs.is_empty() || rhs.is_empty() {
                return None;
            }
            let alias_type = alias_rhs_slot(rhs)?;
            return Some(DirectiveTemplate::CsUsing {
                global_prefix,
                alias_type: Some(alias_type),
                keyword_run,
                target: lane_slot(lhs)?,
            });
        }
        // The plain face: the name is ONE token (whitespace is never a
        // namespace name); the edges are trivia-trimmed. The reference's
        // matcher treats comments as trivia on the PATTERN face too, so
        // comment spans are stripped before the token checks — leftover
        // comment-padding trivia keeps refusing.
        let stripped = strip_comment_spans(inner);
        let had_comment = stripped.len() != inner.len();
        let trimmed = stripped.trim_matches(is_sg_cs_trivia);
        // A comment in a META face keeps the accepted-empty posture —
        // transparency is the LITERAL-face law.
        if had_comment && trimmed.contains('$') {
            return None;
        }
        let inner = trimmed.to_string();
        if inner.is_empty() || inner.chars().any(is_sg_cs_trivia) {
            return None;
        }
        let Some(target) = lane_slot(&inner) else {
            return None;
        };
        return Some(DirectiveTemplate::CsUsing {
            global_prefix,
            keyword_run,
            target,
            alias_type: None,
        });
    }
    None
}

/// The top-level `=` scanner for the cs using-alias face — depth-aware
/// over `(`/`[`/`{` groups, ASCII positions only.
pub(crate) fn top_level_equals(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '=' if depth == 0 => {
                // `==`, `+=`, `=>` never occur in a using directive's name;
                // the first bare `=` is the alias separator.
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// The (template, language) pairing the walk dispatches — the census arm
/// and the dispatch share this exact admission.
pub(crate) fn directive_lane_serves(lang: Language, pattern: &str) -> bool {
    let Some(template) = directive_template(pattern) else {
        return false;
    };
    match (&template, lang) {
        (DirectiveTemplate::Semi { head: "import", .. }, Language::Java) => true,
        (DirectiveTemplate::Semi { head: "use", .. }, Language::Rust | Language::Php) => true,
        (DirectiveTemplate::CsUsing { .. }, Language::CSharp) => true,
        (DirectiveTemplate::PhpUseKind { .. }, Language::Php) => true,
        (
            DirectiveTemplate::PyImport { .. } | DirectiveTemplate::PyFromImport { .. },
            Language::Python,
        ) => true,
        _ => false,
    }
}

pub(crate) fn match_directive_root(
    lang: Language,
    source: &str,
    pattern: &str,
) -> Option<Vec<PatternMatch>> {
    if !directive_lane_serves(lang, pattern) {
        return None;
    }
    let template = directive_template(pattern)?;
    let tree = parse_source(lang, source).ok()?;
    let mut out = Vec::new();
    match &template {
        DirectiveTemplate::PyImport { .. } => {
            walk_kind(tree.root_node(), "import_statement", false, &mut out, |n| {
                py_import_match(n, source, pattern, &template)
            });
        }
        DirectiveTemplate::PyFromImport { .. } => {
            walk_kind(
                tree.root_node(),
                "import_from_statement",
                false,
                &mut out,
                |n| py_from_import_match(n, source, pattern, &template),
            );
        }
        DirectiveTemplate::Semi { head: "import", .. } => {
            walk_kind(
                tree.root_node(),
                "import_declaration",
                false,
                &mut out,
                |n| java_import_match(n, source, pattern, &template),
            );
        }
        DirectiveTemplate::Semi { head: "use", .. } => {
            walk_use_semi(tree.root_node(), source, pattern, &template, lang, &mut out);
        }
        DirectiveTemplate::CsUsing { .. } => {
            walk_kind(tree.root_node(), "using_directive", false, &mut out, |n| {
                cs_using_match(n, source, pattern, &template)
            });
        }
        DirectiveTemplate::PhpUseKind { .. } => {
            walk_use_semi(tree.root_node(), source, pattern, &template, lang, &mut out);
        }
        _ => {}
    }
    Some(out)
}

pub(crate) fn py_import_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    let DirectiveTemplate::PyImport { names, alias } = template else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node
        .children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect();
    if let Some((x, y)) = alias {
        // `import $X as $Y` — the FIRST child must be an aliased_import (a
        // leading plain name refuses); candidate children AFTER it are
        // skipped (a trailing plain name keeps the bind).
        let Some(first) = children.first() else {
            return None;
        };
        if first.kind() != "aliased_import" {
            return None;
        }
        let name_node = first.child_by_field_name("name")?;
        let alias_node = first.child_by_field_name("alias")?;
        x.bind(&mut captures, node_text(&name_node, source)?.trim())?;
        y.bind(&mut captures, node_text(&alias_node, source)?.trim())?;
    } else {
        // The plain shape binds the FIRST child text — an aliased child
        // binds WHOLE (`os as o`, R2_py_importX_alias_cand); candidate
        // extras absorb as a prefix (E_py_import_two n1).
        if children.len() < names.len() {
            return None;
        }
        for (slot, child) in names.iter().zip(children.iter()) {
            slot.bind(&mut captures, node_text(child, source)?.trim())?;
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}

pub(crate) fn py_from_import_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    let DirectiveTemplate::PyFromImport {
        module,
        names,
        star,
        paren_wrapped,
    } = template
    else {
        return None;
    };
    let mut captures = BTreeMap::new();
    if let Some(text) = node_text(node, source) {
        captures.insert("MATCH".to_string(), text.to_string());
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node
        .children(&mut cursor)
        .filter(|c| c.is_named() && !is_trivia_kind(c.kind()))
        .collect();
    // The candidate's parentheses DEMARCATE both ways (the anonymous `(`/`)`
    // tokens are part of the tree; a paren face refuses a paren-free
    // candidate and vice versa).
    let has_paren = node
        .children(&mut cursor)
        .any(|c| !c.is_named() && node_text(&c, source).is_some_and(|t| t == "("));
    if has_paren != *paren_wrapped {
        return None;
    }
    let [module_node, imported @ ..] = children.as_slice() else {
        return None;
    };
    module.bind(&mut captures, node_text(module_node, source)?.trim())?;
    if *star {
        // The star face binds the module only and refuses a named list; a
        // name-SLOT pattern binds the wildcard text under its slot (Y=`*`).
        if imported
            .first()
            .is_some_and(|c| c.kind() != "wildcard_import")
        {
            return None;
        }
    } else if imported.len() < names.len() {
        return None;
    } else {
        for (slot, child) in names.iter().zip(imported.iter()) {
            slot.bind_or_match(&mut captures, node_text(child, source)?.trim())?;
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}

pub(crate) fn java_import_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    let DirectiveTemplate::Semi {
        head: "import",
        static_prefix,
        star_suffix,
        target,
        alias: None,
    } = template
    else {
        return None;
    };
    let text = node_text(node, source)?;
    let rest = text.trim().strip_prefix("import")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let mut rest = rest.trim_start();
    let mut had_static = false;
    if let Some(after) = rest.strip_prefix("static") {
        if after.starts_with(char::is_whitespace) {
            had_static = true;
            rest = after.trim_start();
        }
    }
    if had_static != *static_prefix {
        return None;
    }
    let inner = rest.strip_suffix(';')?;
    let mut had_star = false;
    let mut target_text = inner.trim();
    if let Some(prefix) = target_text.strip_suffix(".*") {
        had_star = true;
        target_text = prefix.trim_end();
    }
    if had_star != *star_suffix {
        return None;
    }
    let mut captures = BTreeMap::new();
    captures.insert("MATCH".to_string(), text.to_string());
    target.bind_or_match(&mut captures, target_text)?;
    Some(hit_for_node(node, source, pattern, captures))
}

pub(crate) fn walk_use_semi(
    node: Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
    lang: Language,
    out: &mut Vec<PatternMatch>,
) {
    let want_kind = match lang {
        Language::Rust => "use_declaration",
        Language::Php => "namespace_use_declaration",
        _ => return,
    };
    if node.kind() == want_kind && !is_in_comment_or_string(&node) {
        if let Some(hit) = use_semi_match(&node, source, pattern, template) {
            out.push(hit);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_use_semi(child, source, pattern, template, lang, out);
    }
}

pub(crate) fn use_semi_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    // The php kind faces — the clause's head token must be the pattern's
    // literal kind keyword; X binds the name (one token: the aliased kind
    // spelling keeps its uncovered route).
    if let DirectiveTemplate::PhpUseKind { kind, target } = template {
        // The kind HEAD matches on the RAW clause text (a comment BEFORE
        // the kind keyword breaks the alignment and refuses); comments
        // AFTER it are trivia — the NAME section scans comment-stripped.
        // The php gap law — after the literal `use` a gap-trivia run is
        // REQUIRED (ASCII ws, NBSP, U+FEFF, U+001A); the kind token ends at
        // the next trivia char, so a Rust-ws outsider (U+2028 et al) glues
        // into the head/name token and refuses exactly where the reference
        // refuses. The name section binds the FIRST token: trivia splits
        // it, glue chars are token chars.
        let raw = node_text(node, source)?;
        let rest = raw.trim().strip_suffix(';')?.strip_prefix("use")?;
        let gap_len: usize = rest
            .chars()
            .take_while(|c| is_sg_php_gap_trivia(*c))
            .map(char::len_utf8)
            .sum();
        if gap_len == 0 {
            return None;
        }
        let after_gap = &rest[gap_len..];
        // NUL ends the head-token scan for Meta targets (the kind clause
        // still aligns); literal targets keep the strict scan.
        let head_end = after_gap
            .find(|c| is_sg_php_gap_trivia(c) || (matches!(target, LaneName::Meta(_)) && c == '\0'))
            .unwrap_or(after_gap.len());
        if &after_gap[..head_end] != &kind[..] {
            return None;
        }
        // The kind-clause NAME gap admits NUL for Meta targets (capture
        // stripped); literal targets keep the strict class.
        let name_gap =
            |c: char| is_sg_php_gap_trivia(c) || (matches!(target, LaneName::Meta(_)) && c == '\0');
        let name = strip_comment_spans(&after_gap[head_end..])
            .trim_start_matches(name_gap)
            .split(name_gap)
            .next()
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            return None;
        }
        let mut captures = BTreeMap::new();
        captures.insert("MATCH".to_string(), raw.to_string());
        target.bind_or_match(&mut captures, &name)?;
        return Some(hit_for_node(node, source, pattern, captures));
    }
    let DirectiveTemplate::Semi {
        head: "use",
        target,
        alias,
        ..
    } = template
    else {
        return None;
    };
    let text = node_text(node, source)?;
    // The php KIND-LESS `use $X;` clause head obeys the same php gap law as
    // the kind lanes — after the literal `use` an is_sg_php_gap_trivia run
    // is REQUIRED (FEFF/A0/001A bind); a glue char (U+2028) means zero gap
    // → the junk glued into the name token and the reference refuses. Rust
    // `use` keeps the literal-space spelling. The use-head gap law admits
    // U+0000 as a gap unit at the META face (`use<NUL>Foo;` binds X=Foo,
    // capture stripped) while the LITERAL pattern keeps refusing the NUL
    // candidate — the admission is target-scoped.
    let meta_target = matches!(target, LaneName::Meta(_));
    let php_clause = node.kind() == "namespace_use_declaration";
    let rest = if php_clause {
        let after_use = text.trim().strip_prefix("use")?;
        let gap_len: usize = after_use
            .chars()
            .take_while(|c| is_sg_php_gap_trivia(*c) || (meta_target && *c == '\0'))
            .map(char::len_utf8)
            .sum();
        if gap_len == 0 {
            return None;
        }
        &after_use[gap_len..]
    } else {
        text.trim().strip_prefix("use ")?
    };
    let inner = rest.strip_suffix(';')?;
    let mut captures = BTreeMap::new();
    captures.insert("MATCH".to_string(), text.to_string());
    if let Some((_, y)) = alias {
        let (left, right) = split_top_level_keyword(inner, " as ")?;
        target.bind_or_match(&mut captures, left.trim())?;
        y.bind(&mut captures, right.trim())?;
    } else {
        // The reference refuses a kind-less META `use $X;` against a php
        // GROUP-use candidate — the meta capture never spans a php brace
        // group — while a literal group-use PATTERN keeps binding its
        // byte-equal candidate and plain candidates keep binding. The rust
        // brace-group face is untouched: the guard is php-clause-scoped.
        if php_clause && inner.contains('{') && matches!(target, LaneName::Meta(_)) {
            return None;
        }
        // The plain shape binds the WHOLE clause text (brace groups,
        // `function foo` kind keywords included: E_rs_use_braces,
        // R3_php_use_function).
        target.bind_or_match(&mut captures, inner.trim())?;
    }
    Some(hit_for_node(node, source, pattern, captures))
}

pub(crate) fn cs_using_match(
    node: &Node,
    source: &str,
    pattern: &str,
    template: &DirectiveTemplate,
) -> Option<PatternMatch> {
    let DirectiveTemplate::CsUsing {
        global_prefix,
        keyword_run,
        target,
        alias_type,
    } = template
    else {
        return None;
    };
    // Candidate-side demarcation: parse the directive's own shape —
    // optional `global`, the `using` keyword, the `static`/`unsafe` keyword
    // run, then a name or `name = type` alias. The reference walks the AST
    // where comments are trivia, so the SCANS see the comment-stripped
    // clause text (and the captures bind trivia-free slot texts) while the
    // emitted span/MATCH keep the original bytes. The demarcation skips
    // are CLASS-STRICT — the CANDIDATE-side class is is_sg_cs_gap_junk
    // (the parse skips full Unicode White_Space + FEFF + control junk
    // here), while the PATTERN-side class of record (directive_template)
    // stays is_sg_cs_trivia.
    let raw = node_text(node, source)?;
    let text = strip_comment_spans(raw);
    let mut rest = text.trim().trim_start_matches(is_sg_cs_gap_junk);
    let mut had_global = false;
    if let Some(after) = rest.strip_prefix("global") {
        if after.starts_with(is_sg_cs_gap_junk) {
            had_global = true;
            rest = after.trim_start_matches(is_sg_cs_gap_junk);
        }
    }
    let Some(after_using) = rest.strip_prefix("using") else {
        return None;
    };
    if !after_using.starts_with(is_sg_cs_gap_junk) {
        return None;
    }
    let mut rest = after_using.trim_start_matches(is_sg_cs_gap_junk);
    let mut had_keywords: Vec<CsUsingKeyword> = Vec::new();
    loop {
        let mut consumed = false;
        for keyword in [CsUsingKeyword::Static, CsUsingKeyword::Unsafe] {
            if let Some(after) = rest.strip_prefix(keyword.spell()) {
                if after.starts_with(is_sg_cs_gap_junk) {
                    had_keywords.push(keyword);
                    rest = after.trim_start_matches(is_sg_cs_gap_junk);
                    consumed = true;
                    break;
                }
            }
        }
        if !consumed {
            break;
        }
    }
    // `global` is demand-only; the keyword run must EQUAL the pattern's.
    if *global_prefix && !had_global {
        return None;
    }
    if *keyword_run != had_keywords {
        return None;
    }
    let inner = rest
        .trim()
        .trim_start_matches(is_sg_cs_gap_junk)
        .strip_suffix(';')?
        .trim_end_matches(is_sg_cs_gap_junk);
    let mut captures = BTreeMap::new();
    captures.insert("MATCH".to_string(), raw.to_string());
    match alias_type {
        Some(alias) => {
            let eq_at = top_level_equals(inner)?;
            let (lhs, rhs) = (
                inner[..eq_at].trim_matches(is_sg_cs_gap_junk),
                inner[eq_at + 1..].trim_matches(is_sg_cs_gap_junk),
            );
            if lhs.is_empty() || rhs.is_empty() {
                return None;
            }
            target.bind_or_match(&mut captures, lhs)?;
            alias.bind_or_match(&mut captures, rhs)?;
        }
        None => {
            // The plain face: one name token. The `=` alias spelling and
            // any decorated run never reach here (the run equality and
            // the trivia guard refuse them). Junk glued inside ONE name
            // token refuses. The law is PER-TOKEN-GAP — junk at an
            // identifier↔`.` boundary is skipped by the parse and the
            // qualified name BINDS with the junk RETAINED in the capture.
            // Segment law: split the top-level `.`, trim each segment's
            // EDGE junk runs, refuse a segment whose INTERIOR carries junk
            // or that empties out; the capture binds the raw
            // (junk-retained) section text.
            if inner.is_empty() || top_level_equals(inner).is_some() {
                return None;
            }
            let cand_segs: Vec<&str> = inner.split('.').collect();
            if cand_segs.iter().any(|seg| {
                let core = seg.trim_matches(is_sg_cs_gap_junk);
                core.is_empty() || core.chars().any(is_sg_cs_gap_junk)
            }) {
                return None;
            }
            // A LITERAL qualified-name pattern obeys the same per-segment law
            // — the reference skips boundary junk and comment gaps on the
            // literal face too, while glue INSIDE one segment refuses. Meta
            // targets keep the junk-retained capture.
            match target {
                LaneName::Meta(_) => {
                    target.bind_or_match(&mut captures, inner)?;
                }
                LaneName::Literal(literal) => {
                    let lit_segs: Vec<&str> = literal.split('.').collect();
                    if lit_segs.len() != cand_segs.len() {
                        return None;
                    }
                    for (seg, lit) in cand_segs.iter().zip(lit_segs.iter()) {
                        if seg.trim_matches(is_sg_cs_gap_junk) != *lit {
                            return None;
                        }
                    }
                }
            }
        }
    }
    Some(hit_for_node(node, source, pattern, captures))
}
