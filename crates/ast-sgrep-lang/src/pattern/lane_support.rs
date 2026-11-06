//! General-lane support predicate (isomorphic split of general_match.rs).

use super::*;
use crate::Language;

/// Language-free: does ANY grammar build a general-lane template?
pub(crate) fn general_lane_supported(pattern: &str) -> bool {
    // The conditional-directive family is natively answerable when the
    // condition parses under the shared [`CondBinding`] rule — lone
    // metavariable, metavar-free text, a single metavariable nested in the
    // condition (`#if defined($A)` ANSWERS like the reference; the preproc
    // lane binds the nested metavar), or a parseable structural condition
    // template over 2+ canonical metavariables. `#define` joins through
    // [`parse_define_tail`]; non-directive patterns fall through untouched.
    if let Some(supported) = preproc_directive_supported(pattern) {
        return supported;
    }
    // The csharp statement-head lane is a language-free "some lane serves
    // this" admission for the ingress gate — the general template cannot root
    // these heads, so the any-language build loop below would report
    // unsupported and `needs_ast_grep_fallback` would refuse faces the
    // reference answers. The lane parse is spelling-level only; the walk +
    // census keep per-file/per-language honesty.
    if matches!(
        pattern.split_whitespace().next(),
        Some("fixed" | "checked" | "unchecked" | "unsafe" | "lock" | "using")
    ) && csharp_statement_template(pattern).is_some()
    {
        return true;
    }
    // The java class member-count lane is a language-free "some lane serves
    // this" admission for the ingress gate — the general template cannot
    // substitute a bare-meta class body (the starvation that kept the face
    // census-loud), so the fallback gate refuses faces the reference
    // answers. The lane parse is spelling-level only; the walk + census
    // keep per-file/per-language honesty.
    if classify_java_class_member_count(pattern).is_some() {
        return true;
    }
    // The directive roots, the remaining statement roots, the java
    // synchronized NESTED/METHOD faces, the php literal-name/`$$`-body
    // namespace faces, the csharp checked/unchecked expression root, and
    // go's `$`-carrying `;`-ful spellings — the language-free "some lane
    // serves this" admissions. The lane parses are spelling-level only;
    // the walk + census keep per-file/per-language honesty.
    if directive_template(pattern).is_some()
        || cs_expression_template(pattern).is_some()
        || kt_typealias_template(pattern).is_some()
        || kt_for_template(pattern).is_some()
        || swift_for_template(pattern).is_some()
        || rs_let_else_template(pattern).is_some()
        || c_goto_template(pattern).is_some()
        || ja_sync_block_nested_template(pattern).is_some()
        || ja_sync_method_template(pattern).is_some()
        || php_namespace_block_template(pattern).is_some()
        || sg_goto_semi_pattern(pattern)
        || sg_go_import_semi_pattern(pattern)
        // The del lane's `$$$`-slot faces ride the dedicated delete template
        // — the general lane's build refuses the multi-meta spelling and
        // would keep the answering face ingress-loud. The census arm stays
        // py-scoped, so other languages keep their loud class.
        || py_delete_meta_pattern(pattern)
    {
        return true;
    }
    if !general_lane_text_eligible(pattern) {
        return false;
    }
    let Some((substituted, placeholders, multi_names)) = substitute_general_metavariables(pattern)
    else {
        return false;
    };
    // Identifier soup (`$A $B`, `RETURN $A`) is not a structural template.
    // Registered statement-head templates (`return $A`) carry no
    // structural punctuation but are still structural — the statement-root
    // template lane decides them; uppercase/unregistered heads never reach
    // here (the bare-keyword guard refuses them first). The head check no
    // longer demands a `$` — the probed `$`-less faces
    // (`break`/`continue`) are statement templates too.
    let statement_head_template = pattern
        .split_whitespace()
        .next()
        .is_some_and(|head| STATEMENT_HEAD_KEYWORDS.contains(&head));
    // A whole-content string meta (`"$A"` — the parsed string-content leaf
    // IS the metavariable) carries no punctuation, but is a structural
    // template whose build the any-language loop below verifies (and whose
    // per-language root-kind admission + whole-content placeholder rule keep
    // `"pre-$A"`/`"$A$B"`/escaped spellings refused).
    let bare_string_meta_template = substituted.len() >= 2
        && substituted.starts_with('"')
        && substituted.ends_with('"')
        && placeholders.len() == 1
        && placeholders.contains_key(&substituted[1..substituted.len() - 1]);
    // A KEYWORD-OPERATOR template (`$X and $Y`, `$X or $Y`, `$X as $Y`,
    // `not $X`, `typeof $X`) carries no punctuation from the structural
    // set — the operator is an alphabetic keyword — but is a structural
    // template whose per-language build the any-language loop below
    // verifies (the root kinds land on the existing
    // `_operator`/`binary`/`_expression` admissions; the `not`/`typeof`
    // heads ride the head admissions above). The unary KEYWORD faces
    // (`delete $X`, `void $X`, `del $X`) join the same lane; their root
    // kinds are admitted in `is_general_root_kind`.
    let keyword_operator_template = !bare_string_meta_template
        && !placeholders.is_empty()
        && pattern.split_whitespace().any(|token| {
            matches!(
                token,
                "and" | "or" | "as" | "not" | "typeof" | "delete" | "void" | "del"
            )
        });
    // The ruby MODIFIER statement template (`x if $C`) carries no
    // structural punctuation but is structural — the rb modifier root kinds
    // are admitted in `is_general_root_kind` and the any-language loop below
    // verifies the build.
    let rb_modifier_template = !placeholders.is_empty() && {
        let tokens: Vec<&str> = pattern.split_whitespace().collect();
        tokens.len() >= 3
            && matches!(
                tokens[tokens.len() - 2],
                "if" | "unless" | "while" | "until"
            )
    };
    if !substituted.contains(|c: char| "()[]{}<>=:,.!?+-*/%&|;~^@#".contains(c))
        && !statement_head_template
        && !bare_string_meta_template
        && !keyword_operator_template
        && !rb_modifier_template
    {
        return false;
    }
    // A `#` whose tail carries NO code structure is a comment-glue tail
    // (`foo($A) # note`, `$A + $A# note`, the python `# tail` ctrl faces)
    // — the registered contract keeps those fail-closed at the
    // language-free ingress even though some grammars happily parse the `#`
    // tail as syntax. The `#`-as-SYNTAX faces (`#[derive($A)]`,
    // `#include $X`, `this.#x = $V`, `tag(r#"$A"#)`) all carry code
    // punctuation after their first `#` and stay templatable.
    if contains_comment_glued_hash(pattern) {
        return false;
    }
    // `#` is comment syntax only in python/ruby/php — the any-language
    // support answer must not count a hash-comment language templating a
    // pattern whose `#` is real syntax elsewhere (and vice versa: rust
    // attribute faces stay supported while python hash-glued faces keep
    // failing closed).
    Language::all().iter().any(|&lang| {
        !lane_comment_refused(lang, pattern)
            && build_general_template(lang, pattern, &substituted, &placeholders, &multi_names)
                .is_some()
    })
}
