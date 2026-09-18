//! NativeKind taxonomy and the native classifier.

use super::*;

/// The parsed `else …` tail of an if template. `ElseIf` nests so a deep
/// `else if … else if …` chain parses recursively (the reference binds every
/// level's captures).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IfAlternative {
    /// `else { B }` / `else: B` — the fallback branch. `body_meta` is the
    /// bare `$NAME` of THIS branch's body section (per-level: the
    /// pattern-global last-section scan cannot serve chained tails).
    Else {
        body: Option<BodyTemplate>,
        body_braced: bool,
        body_meta: Option<String>,
    },
    /// `else if (COND) { B }` — the chained if; `alternative` carries any
    /// further tail. `cond_meta`/`body_meta` are THIS level's bare-meta
    /// captures (None when the section is not a single canonical meta).
    ElseIf {
        cond: Option<String>,
        body: Option<BodyTemplate>,
        body_braced: bool,
        cond_meta: Option<String>,
        body_meta: Option<String>,
        alternative: Option<Box<IfAlternative>>,
    },
}

/// Native structural pattern shapes handled in-process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeKind {
    /// Function-like declaration; `name == None` means any name (`$NAME`).
    /// `body` constrains the statement count of the body (`fn $N($$$) { $STMT }`).
    Function {
        name: Option<String>,
        body: Option<BodyTemplate>,
    },
    /// Class/struct/type declaration. `keyword` is the pattern prefix
    /// (`class`, `struct`, `interface`, or `type`) so queries stay kind-specific.
    /// `body` carries the member-count template for the `interface` prefix
    /// ONLY — the reference binds `interface $N { $B }` against
    /// single-member ts/java interfaces and refuses multi-member
    /// candidates; other grammars keep the census loud (uncovered).
    /// `class`/`struct`/`type` Exactly templates keep their registered
    /// refusal.
    Class {
        keyword: &'static str,
        name: Option<String>,
        body: Option<BodyTemplate>,
    },
    /// Free or method call; method path segments may be `$` wildcards.
    Call {
        /// Exact path like `foo.bar` or single name; segments that were `$X` are None.
        path: Vec<Option<String>>,
        /// Per-position slots when the argument list mixes a `$$$NAME` rest
        /// with positional `$NAME`/`$$NAME` singles (`q($A, $$$B)`) — the
        /// text-derived arity template cannot express it; `None` keeps the
        /// registered empty/`$$$`/pure-singles templates.
        arg_slots: Option<Vec<ArgSlot>>,
    },
    /// The php plain `->` member-call spelling (`$svc->run($A)`). The
    /// reference is connector token-exact — a `->`-spelled callee never
    /// answers `.`/`::`/`?->` call sites and vice versa — so the spelling
    /// gets its own lane over `member_call_expression` candidates with the
    /// full object->name segment chain (raw object text, meta segments
    /// wildcard). Nullsafe candidates: plain spellings never answer them
    /// (token-exact); a pattern that SPELLS `?->` rides this lane with
    /// `nullsafe: true` over `nullsafe_member_call_expression` candidates
    /// (shapes the carve refuses keep the [`NativeKind::OptionalCall`]
    /// lane).
    MemberCall {
        /// Exact path; segments that were `$X` are None.
        path: Vec<Option<String>>,
        /// Per-position slots when the argument list mixes literal tokens
        /// with canonical metas (`$w->q5(1, $B)`). Meta-only flat lists
        /// classify through the slots too (the `;`-terminated spelling had
        /// no answering arm).
        arg_slots: Option<Vec<ArgSlot>>,
        /// Set only for the DANGLING-arrow repair — the repaired pattern never
        /// answers the standalone statement spelling, only chain-prefix sites
        /// whose member-call node is continued by a member-access/call link.
        require_continuation: bool,
        /// The pattern's member connector is the nullsafe `?->` —
        /// candidates switch to `nullsafe_member_call_expression`
        /// (token-exact in both directions, probed reference).
        nullsafe: bool,
    },
    /// The php plain `->` member-call CHAIN spelling with MORE THAN ONE
    /// argument list (`$obj->m1()->m2($A)`). The [`NativeKind::MemberCall`]
    /// lane serves single-call-segment faces only; deeper chains answer
    /// the chain node (and its inner prefix subnodes, visited as their own
    /// member-call nodes). Per-segment contract mirrors
    /// [`NativeKind::CallChain`], but candidates are the php member-call
    /// nodes only — connector token-exact, never a `.` site — and each
    /// call segment's argument list must sit on a REAL call link (a property
    /// access in the receiver chain is not a call). Mid-chain property
    /// segments classify, the nullsafe `?->` connector is spelled per
    /// link, and the LAST segment may be a property link (the walker then
    /// visits member-access candidates too).
    MemberCallChain {
        /// Outermost-first segments. The leading segment is a plain receiver;
        /// later segments are calls or property links (mid-chain or tail).
        segments: Vec<CallChainSegment>,
        /// `nullsafe_flags[j]` is true when the connector INTO segment j is
        /// the nullsafe `?->` (flags[0] always false). Token-exact: a plain
        /// `->` link never answers a `?->` site and vice versa (0.45.2
        /// cross probes, both directions).
        nullsafe_flags: Vec<bool>,
    },
    /// Dotted member-call chains with MORE THAN ONE argument list
    /// (`$O.$M1($$$A).$M2($$$B)`). The reference answers the outermost chain
    /// whose callee path has exactly the pattern's segment count, unifying
    /// per-segment names through metavariable equality (the same-name veto)
    /// and checking every call segment's argument list.
    CallChain {
        /// Outermost-first segments. Only the leading segment may be a
        /// plain receiver; every later segment carries an argument list.
        segments: Vec<CallChainSegment>,
    },
    /// The 3+-segment optional chain `$O?.$M1($$$A).$M2($$$B)`. The spelled
    /// connectors are token-exact per position: `optional_flags[j]` is true
    /// when the connector INTO pattern segment j is the spelled `?.` (the
    /// head link `$O?.…`, the mid-chain spellings `$O.$M1()?.$M2($$$B)`, or
    /// both) and a candidate must match every ALIGNED flag exactly; a
    /// wildcard (or single-token literal) head folds the whole receiver
    /// prefix before the first aligned segment. The two-segment spelling
    /// stays on [`NativeKind::OptionalCall`].
    OptionalCallChain {
        /// `segments[0]` is the folded head; every later segment carries an
        /// argument list (outermost-last).
        segments: Vec<CallChainSegment>,
        /// `optional_flags[j]` = the connector INTO pattern segment j is
        /// `?.` (`flags[0]` is always false — the head has no connector).
        optional_flags: Vec<bool>,
    },
    /// The two-segment optional-chain call spelling `HEAD?.$TAIL(...)`. The
    /// reference treats `?.` as a required anonymous connector token: this
    /// template answers exactly the optional-chain call faces (incl.
    /// wildcard-head folding, `$O` = `user?.profile` on
    /// `user?.profile?.load()`) and never the plain `.` receivers — the
    /// mirror of the plain-template veto. Per-language acceptance is the
    /// reference's own pattern parse (see [`native_kind_language_answerable`]):
    /// grammars where `?` is not a member connector (rust try, python:
    /// none) keep the refusal.
    OptionalCall {
        /// `Some(name)` when the head is the wildcard `$name` (folds any
        /// receiver expression); else the head is a literal identifier.
        head_capture: Option<String>,
        /// Literal head identifier (exact match against the receiver text).
        head_literal: Option<String>,
        /// `Some(name)` when the tail (method) segment is `$name`.
        tail_capture: Option<String>,
        /// Literal tail identifier.
        tail_literal: Option<String>,
    },
    /// `if` statement/expression template: `if ($COND) { $BODY }`,
    /// `if $COND { $BODY }`, or `if $COND: $BODY`. A metavariable condition
    /// rides `cond == None`; CONCRETE conditions ride `cond == Some(text)`
    /// — the text parses as a per-language general-lane expression template
    /// and `general_eq` compares it against the candidate's own condition
    /// node. Paren, brace, and colon forms are normalized so one pattern
    /// matches if-nodes across all indexed languages.
    /// `body_braced` records whether the PATTERN's body was spelled with
    /// braces — brace-ness is STRUCTURAL there (a `{ $B }` pattern answers
    /// only braced-block consequences; a brace-less pattern answers both).
    /// The flag gates `if_body_matches`; a `:`-spelled suite stays
    /// brace-less.
    If {
        cond: Option<String>,
        body: Option<BodyTemplate>,
        body_braced: bool,
        /// The `else { B }` / `else if (Y) { B }` tail. `None` keeps the
        /// registered else-less contract byte-for-byte (a pattern without an
        /// else never refuses an else-carrying candidate).
        alternative: Option<IfAlternative>,
    },
    /// A php assignment/binary pattern whose LHS is a LOWERCASE-LITERAL operand
    /// (byte-exact variable/member/dim text) and whose operator is `=`, an
    /// augmented assignment, or a binary operator, binding the RHS expression
    /// texts. The match span is the `;`-rooted EXPRESSION STATEMENT when
    /// the pattern carries a trailing `;` and the assignment node otherwise;
    /// `;`-terminated patterns bind statement roots only while `;`-less
    /// patterns also answer embedded nodes. A canonical or MixedCase LHS and
    /// any META assignment-target stay out; the lane is PHP-ONLY.
    Assignment {
        /// Byte-exact LHS text (`$alpha`, `$this->prop`, `$eps[0]`).
        target: String,
        /// The spelled operator (`=`, `+=`, `==`, ...).
        op: &'static str,
        /// Which candidate node kind serves the operator.
        op_class: PhpBinaryClass,
        /// When the RHS is NOT one bare meta: the RHS expression pattern
        /// text (parsed at walk time, matched structurally with meta
        /// binding).
        rhs_expr: Option<String>,
        /// RHS capture name for the bare-meta path (`V` for `$V`/`$$V`/`$$$V`).
        value: String,
        /// The bare-meta RHS was spelled `$$$NAME` (multi namespace).
        value_multi: bool,
    },
    /// A pattern carrying a NON-canonical `$`-token — lowercase-LED
    /// (`$a`, `$$$a`) or uppercase/underscore-LED with a lowercase tail
    /// (`$ABc`, `$_a`; the meta-name grammar is `[A-Z_][A-Z0-9_]*`). In
    /// `$`-name languages (js/ts/php-lowercase) such tokens are literal code
    /// instead and route to the literal lane; here they are pattern-tree
    /// ERROR nodes that match nothing (accepted-empty, or exit 8 where
    /// per-language error recovery fails). Valid ingress (ok:true), zero
    /// candidates, never an overmatch.
    NeverMatches,
    /// `$$NAME` / `$$_` universal node metavariable — matches EVERY node
    /// including comments, docstrings,
    /// strings, and anonymous tokens; a named metavariable binds the node
    /// text (`MATCH` reserved-key overwrite semantics preserved).
    Universal { name: Option<String> },
}

pub(crate) fn strip_declaration_modifiers(pattern: &str) -> (&str, Option<&str>) {
    const MODIFIERS: &[&str] = &[
        "export default ",
        "export ",
        "public ",
        "private ",
        "protected ",
        "internal ",
        "abstract ",
        "static ",
        "final ",
        "async ",
        "unsafe ",
        "pub ",
        // The "two-modifier residual" is the two-token SPELLING gap class,
        // not `sealed` alone: every one of these pattern spellings binds
        // on the probed faces (cs `sealed` / `partial` / `public partial`
        // interfaces; java `strictfp` / `sealed` interfaces — all answer
        // N/B against the matching candidate while the subject refused
        // loud: the pattern could not classify Class{interface}). The
        // cross-language faces stay refused here: a ts/java pattern carrying
        // a csharp-only spelling keeps modifiers Some(spelling), so the
        // modifiers match refuses every plain candidate and the ts blanket
        // arm refuses the rest.
        "sealed ",
        "partial ",
        "strictfp ",
    ];

    let original = pattern.trim();
    let mut rest = original;
    loop {
        if let Some(after_pub) = rest.strip_prefix("pub(") {
            if let Some(close) = after_pub.find(") ") {
                let Some(next) = after_pub.get(close + 2..) else {
                    break;
                };
                rest = next;
                continue;
            }
        }
        // A java ANNOTATION leads the modifiers list (`@Deprecated public
        // interface …`) and the pattern-side modifiers include it — the
        // annotation-carrying PATTERN faces bind when the candidate carries
        // the same annotation (`@Deprecated public interface $N { $B }` ×
        // `@Deprecated interface K`; `@Deprecated` alone ⊆ `@Deprecated
        // @SafeVarargs`) and refuse otherwise. Consume `@ident` optionally
        // followed by ONE balanced `(...)` group plus trailing whitespace;
        // an unterminated group keeps the face's fail-closed class (no
        // strip).
        if let Some(after_at) = rest.strip_prefix('@') {
            let ident_len = after_at
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .map(char::len_utf8)
                .sum::<usize>();
            let annotation_len = if ident_len == 0 {
                None
            } else if let Some(after_paren) = after_at[ident_len..].strip_prefix('(') {
                // `@Ident(…)` — the arg group must balance to strip.
                balanced_paren_close(after_paren).map(|close| ident_len + 1 + close + 1)
            } else {
                Some(ident_len)
            };
            if let Some(len) = annotation_len {
                rest = after_at[len..].trim_start();
                continue;
            }
        }
        let Some(next) = MODIFIERS
            .iter()
            .find_map(|modifier| rest.strip_prefix(*modifier))
        else {
            break;
        };
        rest = next;
    }
    let modifiers = original.strip_suffix(rest).unwrap_or_default().trim();
    (rest, (!modifiers.is_empty()).then_some(modifiers))
}

/// Classify a metavariable / structural pattern into the native subset.
/// The pre-gate carve for a FLAT php `->` member call whose argument list
/// carries slot-worthy shapes (mixed literal/meta lists, whole-list rests,
/// meta-only lists and the `;`-terminated nullsafe spelling). Every other
/// shape returns None so its registered route stays untouched: non-member
/// callees keep the plain Call lane, no-semi nullsafe spellings keep the
/// OptionalCall lane, and a slot grammar refusal (MixedCase tokens,
/// composite literals) falls through to the NeverMatches gate.
/// Single-call discipline mirrors `literal_variable_callee_admitted`: the
/// one argument list must close the pattern with no nested parens.
pub(crate) fn classify_member_call_mixed_args(p: &str) -> Option<NativeKind> {
    // The `;`-terminated flat spelling is the SAME face (the reference
    // answers `$w->q9(1, $$$A);` identically to the bare spelling) —
    // strip the terminator the way the assignment lane does instead of
    // letting the `)`-close check refuse it. Whether the `;` rode the
    // pattern decides the nullsafe admission below.
    let p = p.trim();
    let had_semi = p.ends_with(';');
    let p = p.strip_suffix(';').unwrap_or(p).trim();
    let open = p.find('(')?;
    let close = p.rfind(')')?;
    if close <= open || !p[close + 1..].trim().is_empty() {
        return None;
    }
    if p[open + 1..close].contains(['(', ')']) {
        return None;
    }
    let args = p[open + 1..close].trim();
    // Whole-list rest `$$$A`: the chained spelling answers it (Any arity +
    // whole-list capture) while the post-gate arm never classifies a
    // non-canonical receiver (gate NeverMatches), so the flat spelling
    // silent-emptied reference-answering faces (the flat/chained duopoly;
    // probed `$w->q9($$$A);` answers every arity incl. empty).
    // META-ONLY lists (`$A`, `$A, $B`, same-name `$A, $A`) classify
    // through the slots too — the `;`-terminated flat spelling never
    // reached an answering arm before (routed to the post-gate arm, whose
    // `;`-tail carve refuses → gate NeverMatches silent-[]) while the
    // reference answers the statement-root sites (probed `$o->c1($A);`
    // {2}, `$o?->c1($A);` {3}, `$o->c1($A, $B);` {4}; the same-name list
    // answers EQUAL args only — bind_capture's conflict veto). Empty and
    // bare-`$$$` lists keep their own routes.
    let whole_rest = args
        .strip_prefix("$$$")
        .filter(|name| is_metavar_name(name));
    let meta_only = !args.is_empty() && args != "$$$" && validate_argument_pattern(args).is_some();
    // An EMPTY argument list classifies when the callee's TAIL segment is a
    // pure metavariable — the reference answers `$o->$M();` and the
    // `;`-terminated nullsafe `$o?->$M();`. Literal-name empty calls keep
    // their registered routes (`$o?->m1()` via OptionalCall).
    let callee = p[..open].trim();
    // The nullsafe spelling normalizes to the plain connector for the path
    // check (`?->` splits below; `parse_call_path` refuses `?` bytes).
    let callee_plain = callee.replace("?->", "->");
    let empty_meta_call = args.is_empty()
        && callee_plain.contains("->")
        && parse_call_path(&callee_plain)
            .and_then(|path| path.last().cloned())
            .is_some_and(|last| last.is_none());
    if whole_rest.is_none()
        && validate_argument_pattern(args).is_some()
        && !meta_only
        && !empty_meta_call
    {
        return None;
    }
    if !callee.contains("->") {
        return None;
    }
    // The php nullsafe connector rides BETWEEN the receiver and the call
    // (`$o?->c1`). The `;`-TERMINATED spelling admits here (the semi spelling
    // keeps the statement-root discipline — the chained-embedded inner call
    // stays unanswered). The `;`-LESS nullsafe spellings keep their
    // registered OptionalCall routes untouched — the carve refuses them
    // (fail-closed, like parse_call_path's own `?`-refusal) and the gate lets
    // them through exactly as before.
    let (callee, nullsafe) = match callee.split_once("?->") {
        Some((head, tail))
            if had_semi && !head.is_empty() && !head.contains('?') && !tail.contains('?') =>
        {
            (alloc_member_callee(head, tail), true)
        }
        _ => {
            if callee.contains('?') {
                return None;
            }
            (callee.to_string(), false)
        }
    };
    let slots = if let Some(name) = whole_rest {
        vec![ArgSlot::Rest(name.to_string())]
    } else if args.is_empty() {
        // The empty meta-name call binds zero arguments (`$o->$M();` answers
        // the zero-arity sites).
        Vec::new()
    } else {
        parse_member_arg_slots(args)?
    };
    let path = parse_call_path(&callee)?;
    Some(NativeKind::MemberCall {
        path,
        arg_slots: Some(slots),
        require_continuation: false,
        nullsafe,
    })
}

/// Rejoin the nullsafe split (`head?->tail` → `head->tail`) so the plain
/// dotted-path parser serves the nullsafe spelling.
pub(crate) fn alloc_member_callee(head: &str, tail: &str) -> String {
    format!("{head}->{tail}")
}

/// True when `pattern` classifies into one of the two php
/// comment-TRANSPARENT faces — the assignment-hook lane or the bare php
/// operand-template lane. Both lanes decide comment-carrying spellings by
/// AST structure (the comment is trivia), so core's NeverMatches exemption
/// may hand them to the walk. Every OTHER comment-carrying spelling —
/// notably a member-call pattern whose comment sits in an ARGUMENT slot
/// (the slot grammar refuses comment trivia, so the structural arm answers
/// empty) — is NOT walk-decidable and must keep its census-loud
/// fail-closed class.
pub fn php_comment_transparent_operand_lane(pattern: &str) -> bool {
    classify_php_assignment(pattern).is_some() || classify_php_operand_template(pattern).is_some()
}

pub fn classify_native(pattern: &str) -> Option<NativeKind> {
    let p = pattern.trim();
    // `if` templates first: `if ($COND)` must never classify as a call to `if`.
    if is_if_prefixed(p) {
        return classify_if_template(p);
    }
    // A strict php `->` member-call CHAIN (≥2 call segments, canonical-meta
    // or identifier call heads, pure-metavar argument lists — see
    // [`classify_member_call_chain`]) classifies BEFORE the non-canonical
    // gate: the reference answers `$obj->m1()->m2($A)` on the lowercase-led
    // receiver's literal-source reading, while the carve below deliberately
    // refuses multi-call shapes (its single-call discipline) — without this
    // pre-gate arm every chain face with a `$variable` receiver landed in
    // NeverMatches and answered silent `ok:true []`. MixedCase/garbage
    // `$`-receivers refuse inside the classifier (php poisons the whole
    // pattern) and keep the gate's NeverMatches class; every other refusal
    // falls through to the gate and the historical arms unchanged.
    if let Some(chain) = classify_member_call_chain(p) {
        return Some(chain);
    }
    // A flat php `->` member call whose argument list MIXES literal tokens
    // with canonical metas classifies BEFORE the non-canonical gate — the
    // same genus as the chain carve above (the reference answers
    // `$w->q5(1, $B)` under the lowercase receiver's literal-source reading,
    // and the carve below the gate refuses exactly these lists:
    // `literal_variable_callee_admitted` demands a pure-metavar argument
    // list). Meta-only lists keep the registered post-gate arm,
    // all-lowercase faces keep `dollar_literal_lane`, and a refused slot
    // grammar (MixedCase anywhere) falls through to the gate's NeverMatches
    // class unchanged.
    if let Some(kind) = classify_member_call_mixed_args(p) {
        return Some(kind);
    }
    // A non-canonical `$`-token anywhere in the pattern (lowercase-led, or an
    // uppercase/underscore-led name with a lowercase tail) is not a
    // canonical metavariable — the pattern tree carries an ERROR node that
    // matches nothing. Route the WHOLE pattern to NeverMatches before any
    // shape-specific arm can wildcard it.
    // EXCEPT a 1-dollar LOWERCASE-led token, which is php variable / js-ts
    // identifier SYNTAX, not a poisoned meta. When every non-canonical
    // token in the pattern is such a token AND each sits as a complete
    // callee-head segment (`::` / `->` / `.`-delimited), the pattern keeps
    // the structural call lanes with those segments matched as literal
    // text; the all-lowercase faces keep the literal lane, and mixed-case
    // or 2/3-dollar classes keep NeverMatches.
    // the dedicated optional lane classifies it.
    if pattern_has_noncanonical_metavar(p) && !literal_variable_callee_admitted(p) {
        return Some(NativeKind::NeverMatches);
    }
    // `$$NAME` / `$$_` — the reference's universal node metavariable.
    // `$$`-led shapes that are not a canonical name (`$$`, `$$a`, `$$3`)
    // keep today's loud-reject class (registered residuals).
    if let Some(rest) = p.strip_prefix("$$") {
        return is_metavar_name(rest).then(|| NativeKind::Universal {
            name: (rest != "_").then(|| rest.to_string()),
        });
    }
    let (declaration, _) = strip_declaration_modifiers(p);
    for &(prefix, is_class) in DECL_PATTERN_PREFIXES {
        if let Some(rest) = declaration.strip_prefix(prefix) {
            let head = rest
                .split(|c: char| c == '(' || c == '{' || c == '<' || c == ':' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .trim();
            if head.is_empty() {
                return None;
            }
            // Declaration-head three-way: a canonical metavariable head
            // stays a wildcard name; a garbage-led head
            // (`$3.14`, `$0x1F`) is a loud reject — never the wildcard
            // empty-body acceptance hole. Lowercase-led heads never reach
            // here (the NeverMatches gate above already routed them).
            let name = match head.strip_prefix('$').filter(|rest| !rest.starts_with('$')) {
                Some(metavar) if is_metavar_name(metavar) => None,
                Some(_) => return None,
                None if is_pattern_ident(head) => Some(head.to_string()),
                None => return None,
            };
            let tail = rest[head.len()..].trim();
            // `type $N = { $B }` — the ts alias-OBJECT face joins the
            // member-count lane under a dedicated `type-alias` keyword (its
            // query row lands on type_alias_declaration, never the class rows
            // the plain `type` keyword serves). Brace-less alias tails
            // (`type $N = $V`) stay unclassified here and ride the general
            // lane, whose type_alias_declaration root binds V exactly.
            let (tail, class_keyword) = if prefix.trim() == "type" {
                match tail.strip_prefix('=') {
                    Some(after) => (after.trim(), "type-alias"),
                    None => (tail, "type"),
                }
            } else {
                (tail, prefix.trim())
            };
            let body = if is_class {
                parse_body_template(tail)?
            } else {
                parse_function_tail(tail)?
            };
            return Some(if is_class {
                // Statement-count templates on type bodies are language-specific
                // (fields vs methods); only `{ $$$ }` / no body are supported
                // — EXCEPT the `interface` prefix, which admits the
                // member-count template (binds on single-member ts/java
                // interfaces), scoped to the receipted grammars at the
                // answerability gate and the walk. The ts `type-alias` object
                // face joins with the same discipline (1-member binds,
                // 2-member refuses). The plain `class` prefix KEEPS the
                // refusal here — the bare-colon empty-suite contract rides it
                // for every language — and the JAVA member-count face
                // classifies through [`classify_java_class_member_count`].
                if matches!(body, Some(BodyTemplate::Exactly(_)))
                    && !matches!(class_keyword, "interface" | "type-alias")
                {
                    return None;
                }
                NativeKind::Class {
                    keyword: class_keyword,
                    name,
                    body,
                }
            } else {
                NativeKind::Function { name, body }
            });
        }
    }

    // Calls: $F($$$), foo($$$), $O.$M($$$), a.b.$$$c($$$) — and dotted
    // chains carrying MORE THAN ONE argument list
    // (`$O.$M1($$$A).$M2($$$B)`), which the single-argument-list call shape
    // below cannot spell (its `(`..`)` slice spans both lists).
    if let Some(chain) = classify_call_chain(p) {
        return Some(chain);
    }
    let open = p.find('(')?;
    let close = p.rfind(')')?;
    if close <= open {
        return None;
    }
    // Allow trailing whitespace only after the closing paren.
    if close + 1 != p.len() && !p[close + 1..].trim().is_empty() {
        return None;
    }
    let args = p[open + 1..close].trim();
    let callee = p[..open].trim();
    if callee.is_empty() {
        return None;
    }
    // A `->`-spelled member call may carry a MIXED literal/meta argument
    // list; every other call shape keeps the meta-only argument contract.
    let arg_slots = if callee.contains("->") {
        match validate_argument_pattern(args) {
            Some(()) => None,
            None => {
                let slots = parse_member_arg_slots(args)?;
                Some(slots)
            }
        }
    } else {
        // A plain call whose argument list mixes a `$$$NAME` rest with
        // positional singles classifies through the per-position slot
        // grammar — the reference answers the rest-slot shapes uniformly
        // (trailing rest binds >= 1, non-trailing rest binds exactly zero,
        // two rests split freely) and the general lane refuses every
        // sibling-rest list. Sole rests and pure-single lists keep the
        // registered templates; literal tokens and unprobed shapes (two
        // rests with singles) stay unclassified here exactly as before.
        match validate_argument_pattern(args) {
            Some(()) => None,
            None => parse_call_arg_slots(args).map(Some)?,
        }
    };
    if let Some(path) = parse_call_path(callee) {
        // A `->`-spelled callee is the php member-call lane — connector
        // token-exact, never the plain Call lane. This arm is plain-`->`
        // only: parse_call_path refuses the `?`-carrying nullsafe spelling
        // (it keeps the dedicated optional lane below, and the pre-gate
        // carve serves its slot faces with `nullsafe: true`).
        if callee.contains("->") {
            return Some(NativeKind::MemberCall {
                path,
                arg_slots,
                require_continuation: false,
                nullsafe: false,
            });
        }
        return Some(NativeKind::Call { path, arg_slots });
    }
    // The optional-chain spelling `HEAD?.$TAIL` (exactly one `?.`
    // connector, two segments). Deeper optional chains and `?.` inside
    // multi-call chains have no contract and stay fail-closed here.
    let (head_capture, head_literal, tail_capture, tail_literal) =
        parse_optional_call_path(callee)?;
    Some(NativeKind::OptionalCall {
        head_capture,
        head_literal,
        tail_capture,
        tail_literal,
    })
}
