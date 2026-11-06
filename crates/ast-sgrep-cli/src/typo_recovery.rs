use crate::Cli;
use clap::CommandFactory;

const KNOWN_LONG_FLAGS: &[&str] = &[
    "json",
    "robot-help",
    "root",
    "limit",
    "index-path",
    "lang",
    "durability",
    "no-auto-index",
    "auto-index",
    "no-embed",
    "neural-embed",
    "semantic-only",
    "tantivy",
    "ann-threshold",
    "ann-probes",
    "rerank",
    "rerank-top-k",
    "format",
    "preview",
    "excerpt-lines",
    "snippet-tokens",
    "response-snippet-tokens",
    "file-filter",
    "files-with-matches",
    "count",
    "path",
    "budget-tokens",
    "help",
    "version",
    "dry-run",
    "pattern",
    "rewrite",
    "max-depth",
    "max-nodes",
    "max-edges",
    "max-work",
    "query",
    "iterations",
    "suite",
    "fixture",
    "queries-file",
    "skip-index",
    "debounce-ms",
    "scip",
    "robot-triage",
    "robot-next",
    "requests",
    "yes",
    "force",
    "target",
];

const COLOR_NOOPS: &[&str] = &["color", "colour", "no-color", "no-colour"];

fn closest_long_flag(name: &str) -> Option<&'static str> {
    if let Some(flag) = KNOWN_LONG_FLAGS.iter().copied().find(|flag| *flag == name) {
        return Some(flag);
    }
    KNOWN_LONG_FLAGS
        .iter()
        .copied()
        .map(|flag| (flag, edit_distance(name, flag)))
        .min_by_key(|(_, distance)| *distance)
        .filter(|(flag, distance)| {
            is_adjacent_transposition(name, flag)
                || *distance <= 1
                || (*distance == 2
                    && name.len().abs_diff(flag.len()) <= 1
                    && name.len().min(flag.len()) >= 4)
        })
        .map(|(flag, _)| flag)
}

fn inject_robot_triage(
    raw: Vec<std::ffi::OsString>,
    warnings: &mut Vec<String>,
) -> Vec<std::ffi::OsString> {
    let has_doctor = raw
        .iter()
        .take_while(|arg| arg.as_os_str() != "--")
        .any(|arg| arg == "doctor");
    let mut has_triage = false;
    let mut passthrough = false;
    let mut rewritten = Vec::with_capacity(raw.len() + 1);
    for arg in raw {
        if arg == "--" {
            passthrough = true;
        }
        if passthrough {
            rewritten.push(arg);
            continue;
        }
        let text = arg.to_str().unwrap_or("");
        if matches!(text, "--robot-next" | "--robot-diagnose") {
            warnings.push(
                "note: recovered mega-command as `--robot-triage`. Next time: asgrep doctor --robot-triage"
                    .into(),
            );
            rewritten.push(std::ffi::OsString::from("--robot-triage"));
            has_triage = true;
            continue;
        }
        if text == "--robot-triage" {
            has_triage = true;
        }
        rewritten.push(arg);
    }
    if has_triage && !has_doctor && !rewritten.is_empty() {
        rewritten.insert(1, std::ffi::OsString::from("doctor"));
    }
    rewritten
}

/// Commands a typo may be silently recovered INTO: read-only info commands,
/// plus any command when the argv carries explicit help intent (the recovered
/// run cannot mutate anything). Mutating commands (index, reindex, codemod…)
/// are excluded so a near-miss token never performs writes.
const RECOVERABLE_INFO_COMMANDS: &[&str] = &[
    "status",
    "capabilities",
    "version",
    "robot-docs",
    "doctor",
    "read",
];

fn positional_recovery_is_safe(canonical: &str, help_intent: bool) -> bool {
    RECOVERABLE_INFO_COMMANDS.contains(&canonical) || help_intent
}

/// grep-style reachability habit: `chain SYMBOL --to TARGET` is the call-path
/// query (SOURCE SINK) in different clothing. Recover the canonical shape
/// only when it is unambiguous (global flags may precede the verb; at most
/// one positional; `--to` target last or attached); anything else keeps the
/// chain verb intact so clap's own errors still name the real flags.
fn recover_chain_to_call_path(
    parser: &clap::Command,
    raw: &[std::ffi::OsString],
) -> Option<Vec<std::ffi::OsString>> {
    // Locate the `chain` verb: skip argv[0], global flags, and flag values
    // (the parser knows which long flags take values).
    let mut chain_at = None;
    let mut expect_value = false;
    for (position, arg) in raw.iter().enumerate().skip(1) {
        let text = arg.to_str()?;
        if expect_value {
            expect_value = false;
            continue;
        }
        if let Some(rest) = text.strip_prefix("--") {
            if !text.contains('=') {
                let name = rest.split('=').next().unwrap_or(rest);
                expect_value = parser
                    .get_arguments()
                    .any(|spec| spec.get_long() == Some(name) && spec.get_action().takes_values());
            }
            continue;
        }
        if text.starts_with('-') && text != "-" {
            continue;
        }
        if text == "chain" {
            chain_at = Some(position);
        }
        break;
    }
    let start = chain_at? + 1;
    let mut flags = Vec::new();
    let mut query: Option<std::ffi::OsString> = None;
    let total = raw.len();
    let mut index = start;
    while index < total {
        let text = raw[index].to_str()?;
        if text == "--to" {
            if index + 1 >= total {
                return None;
            }
            let query = query?;
            let mut out = raw[..start - 1].to_vec();
            out.push(std::ffi::OsString::from("call-path"));
            out.extend(flags);
            out.push(query);
            out.push(raw[index + 1].clone());
            out.extend(raw[index + 2..].iter().cloned());
            return Some(out);
        }
        if let Some(target) = text.strip_prefix("--to=") {
            let query = query?;
            let mut out = raw[..start - 1].to_vec();
            out.push(std::ffi::OsString::from("call-path"));
            out.extend(flags);
            out.push(query);
            out.push(std::ffi::OsString::from(target));
            out.extend(raw[index + 1..].iter().cloned());
            return Some(out);
        }
        if text.starts_with('-') && text != "-" {
            flags.push(raw[index].clone());
        } else if query.is_none() {
            query = Some(raw[index].clone());
        } else {
            // A second positional is a ROOT — the intent is not the simple
            // reachability habit; leave the argv alone.
            return None;
        }
        index += 1;
    }
    None
}

/// Recover Levenshtein-1 / transposition flag and subcommand typos before clap.
pub(crate) fn rewrite_typos(
    raw: Vec<std::ffi::OsString>,
) -> (Vec<std::ffi::OsString>, Vec<String>) {
    let mut warnings = Vec::new();
    let raw = inject_robot_triage(raw, &mut warnings);
    let help_intent = raw
        .iter()
        .skip(1)
        .take_while(|arg| arg.as_os_str() != "--")
        .any(|arg| matches!(arg.to_str(), Some("-h") | Some("--help")));
    let parser = Cli::command();
    let raw = match recover_chain_to_call_path(&parser, &raw) {
        Some(rewritten) => {
            warnings.push(
                "note: recovered `chain <sym> --to <target>` onto `call-path <source> <sink>`. \
                 Next time: asgrep call-path <source> <sink>"
                    .to_string(),
            );
            rewritten
        }
        None => raw,
    };
    let mut out = Vec::with_capacity(raw.len());
    let mut passthrough = false;
    let mut saw_positional = false;
    // ast-grep batch habit (`sg run -p PATTERN ROOT`): when argv was routed
    // onto search BY RECOVERY (bridge, -p, or a run/scan vocab reroute) and
    // the query rides --pattern flags, the first bare positional is the ROOT.
    // Explicit `codemod --pattern P .` / `search --pattern P QUERY` keep their
    // own positional grammar (the search help pins QUERY as the slot).
    let mut search_via_recovery = false;
    let mut pattern_seen = false;
    let mut pattern_root_seen = false;
    // grep `-w WORD` habit: the word-boundary mode maps onto the `word:`
    // query channel; the prefix applies to the first surviving QUERY
    // positional, never after a non-search subcommand was named.
    let mut word_prefix = false;
    let mut word_prefixed = false;
    let mut nonsearch_subcommand = false;
    let mut value_pending = false;
    let mut inserted_search_subcommand = false;
    let mut index = 0;
    while index < raw.len() {
        let arg = raw[index].clone();
        let next_text = raw.get(index + 1).and_then(|next| next.to_str());
        index += 1;
        if index == 1 || passthrough {
            out.push(arg);
            continue;
        }
        if value_pending {
            value_pending = false;
            out.push(arg);
            continue;
        }
        if arg == "--" {
            passthrough = true;
            out.push(arg);
            continue;
        }
        let Some(text) = arg.to_str() else {
            out.push(arg);
            continue;
        };
        if let Some(rest) = text.strip_prefix("--") {
            let (name, value) = match rest.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (rest, None),
            };
            let folded = name.to_ascii_lowercase();
            if COLOR_NOOPS.contains(&folded.as_str()) {
                warnings.push(format!(
                    "note: ignored `--{name}`; asgrep is monochrome. Use NO_COLOR=1. Machine data: asgrep --json …"
                ));
                continue;
            }
            const JSON_ALIASES: &[&str] = &["machine", "output-json", "output_json"];
            if JSON_ALIASES.contains(&folded.as_str())
                || (folded == "format" && value.is_some_and(|v| v.eq_ignore_ascii_case("json")))
            {
                warnings.push(format!(
                    "note: recovered `--{rest}` as `--json`. Next time: --json"
                ));
                out.push(std::ffi::OsString::from("--json"));
                continue;
            }
            // `--format json` (detached value): recover before the enum rejects it.
            if folded == "format" && value.is_none() && next_text == Some("json") {
                warnings.push(
                    "note: recovered `--format json` as `--json`. Next time: --json".to_string(),
                );
                out.push(std::ffi::OsString::from("--json"));
                index += 1; // consume the detached `json` value
                continue;
            }
            // `--json=<anything>`: --json takes no value; the envelope is
            // already pretty-printed, so any value spelling is stripped.
            if folded == "json" && value.is_some() {
                warnings.push(format!(
                    "note: recovered `--{rest}` as `--json` (--json takes no value; \
                     the envelope is already pretty-printed). Next time: --json"
                ));
                out.push(std::ffi::OsString::from("--json"));
                continue;
            }
            // grep/rg habit vocabulary with an exact asgrep equivalent.
            if let Some((canonical, takes_value)) = habit_long_flag(&folded) {
                let rewritten = match value {
                    Some(value) => format!("--{canonical}={value}"),
                    None => format!("--{canonical}"),
                };
                warnings.push(format!(
                    "note: recovered `--{name}` as `--{canonical}`. Next time: {rewritten}"
                ));
                value_pending = value.is_none() && takes_value;
                out.push(std::ffi::OsString::from(rewritten));
                continue;
            }
            // grep/rg habit vocabulary that is already the default: accept and teach.
            if let Some(note) = habit_noop_long_flag(&folded) {
                warnings.push(format!("note: ignored `--{name}`; {note}"));
                continue;
            }
            // ast-grep habit: a root-level `--pattern` belongs to the search
            // subcommand. Recover by routing argv onto `search` (exact
            // semantics, so this is an alias-grade recovery, not a rejection).
            if folded == "pattern" && !saw_positional && !inserted_search_subcommand {
                warnings.push(
                    "note: recovered root-level `--pattern` onto the search subcommand. \
                     Next time: asgrep search --pattern <pattern>"
                        .to_string(),
                );
                out.insert(1, std::ffi::OsString::from("search"));
                inserted_search_subcommand = true;
                search_via_recovery = true;
            }
            let canonical = closest_long_flag(&folded);
            let option_name = canonical.map_or(folded.as_str(), |name| name);
            value_pending = value.is_none()
                && parser.get_arguments().any(|spec| {
                    spec.get_long() == Some(option_name) && spec.get_action().takes_values()
                });
            if let Some(canonical) = canonical {
                if canonical != folded {
                    let rewritten = match value {
                        Some(value) => format!("--{canonical}={value}"),
                        None => format!("--{canonical}"),
                    };
                    warnings.push(format!(
                        "note: recovered `--{name}` as `--{canonical}`. Next time: {rewritten}"
                    ));
                    out.push(std::ffi::OsString::from(rewritten));
                    continue;
                }
            }
            if folded == "pattern" {
                pattern_seen = true;
                // --pattern is a search-level flag, so the root-level parser
                // spec cannot tell us it takes a value; mark it explicitly or
                // the pattern value would be mistaken for a positional.
                if value.is_none() {
                    value_pending = true;
                }
            }
            out.push(arg);
            continue;
        }
        if text.starts_with('-') && text != "-" {
            // ast-grep habit: bare `-p PATTERN` is the same surface as the
            // repeatable `--pattern`. Recover the exact short form only;
            // clusters containing `p` keep failing intact so clap's error
            // names the whole token.
            // grep/rg context habits: `-A N` / `-B N` (and attached `-A3`)
            // map onto the single window knob `--excerpt-lines N`. The
            // window opens AROUND each hit — after/before are not separate
            // knobs, and the note says so. Attached forms must be fully
            // numeric (`-A3`); anything else fails intact so clap names the
            // whole token.
            // grep `-m N` (max-count) habit: asgrep has no per-file cap —
            // queries are fused, not per-file regex — but the global hit cap
            // `--limit N` is the nearest exact knob. Same approximation
            // class as `-A`/`-B`: recover with the note naming the
            // difference.
            if text == "-m" {
                warnings.push(
                    "note: recovered `-m` as `--limit` (global hit cap; asgrep has no \
                     per-file match cap — queries are fused). Next time: --limit N"
                        .to_string(),
                );
                value_pending = true;
                out.push(std::ffi::OsString::from("--limit"));
                continue;
            }
            if let Some(count) = text
                .strip_prefix("-m")
                .filter(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
            {
                warnings.push(format!(
                    "note: recovered `{text}` as `--limit {count}` (global hit cap; asgrep has \
                     no per-file match cap — queries are fused). Next time: --limit {count}"
                ));
                out.push(std::ffi::OsString::from(format!("--limit={count}")));
                continue;
            }
            if text == "-A" || text == "-B" {
                warnings.push(format!(
                    "note: recovered `{text}` as `--excerpt-lines` (one window around each \
                     hit; after/before are not separate knobs). Next time: --excerpt-lines N"
                ));
                value_pending = true;
                out.push(std::ffi::OsString::from("--excerpt-lines"));
                continue;
            }
            if let Some(count) = text
                .strip_prefix("-A")
                .or_else(|| text.strip_prefix("-B"))
                .filter(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
            {
                warnings.push(format!(
                    "note: recovered `{text}` as `--excerpt-lines {count}`. Next time: \
                     --excerpt-lines {count}"
                ));
                out.push(std::ffi::OsString::from(format!("--excerpt-lines={count}")));
                continue;
            }
            if text == "-p" {
                warnings.push(
                    "note: recovered `-p` as `--pattern`. Next time: asgrep search --pattern <pattern>"
                        .to_string(),
                );
                if !saw_positional && !inserted_search_subcommand {
                    out.insert(1, std::ffi::OsString::from("search"));
                    inserted_search_subcommand = true;
                    search_via_recovery = true;
                }
                pattern_seen = true;
                value_pending = true;
                out.push(std::ffi::OsString::from("--pattern"));
                continue;
            }
            // grep/rg habit short flags: recover clusters of known letters.
            if let Some(cluster) = text.strip_prefix('-') {
                if !cluster.is_empty()
                    && !cluster.starts_with('-')
                    && cluster.chars().all(|c| HABIT_SHORT_LETTERS.contains(&c))
                {
                    let letters: Vec<char> = cluster.chars().collect();
                    // A cluster containing a mode-gap letter (-w -F -v -c -m)
                    // must fail intact so clap's error names the whole token;
                    // augment teaches the gap instead of the value tip.
                    if !letters.iter().any(|c| MODE_GAP_SHORTS.contains(c)) {
                        let last = letters.len() - 1;
                        for (position, letter) in letters.iter().enumerate() {
                            match letter {
                                'l' => {
                                    warnings.push(
                                        "note: recovered `-l` as `--files-with-matches`. Next time: --files-with-matches"
                                            .to_string(),
                                    );
                                    out.push(std::ffi::OsString::from("--files-with-matches"));
                                }
                                'w' => {
                                    warnings.push(
                                        "note: recovered `-w` as the `word:` query channel. \
                                         Next time: asgrep word:QUERY"
                                            .to_string(),
                                    );
                                    word_prefix = true;
                                }
                                'F' => {
                                    warnings.push(
                                        "note: ignored `-F`; queries are grammar tokens, \
                                         never regex, so fixed-string is the default"
                                            .to_string(),
                                    );
                                }
                                'c' => {
                                    warnings.push(
                                        "note: recovered `-c` as `--count`. Next time: --count"
                                            .to_string(),
                                    );
                                    out.push(std::ffi::OsString::from("--count"));
                                }
                                'C' if position == last => {
                                    warnings.push(
                                        "note: recovered `-C` as `--excerpt-lines`. Next time: --excerpt-lines N"
                                            .to_string(),
                                    );
                                    value_pending = true;
                                    out.push(std::ffi::OsString::from("--excerpt-lines"));
                                }
                                'g' if position == last => {
                                    warnings.push(
                                        "note: recovered `-g` as `--file-filter`. Next time: --file-filter <glob>"
                                            .to_string(),
                                    );
                                    value_pending = true;
                                    out.push(std::ffi::OsString::from("--file-filter"));
                                }
                                't' if position == last => {
                                    warnings.push(
                                        "note: recovered `-t` as `--lang`. Next time: --lang <lang>"
                                            .to_string(),
                                    );
                                    value_pending = true;
                                    out.push(std::ffi::OsString::from("--lang"));
                                }
                                'e' if position == last => {
                                    warnings.push(
                                        "note: recovered `-e` as the positional QUERY. Next time: asgrep search <query>"
                                            .to_string(),
                                    );
                                    value_pending = true; // the pattern passes through as QUERY
                                }
                                'i' => warnings.push(
                                    "note: ignored `-i`; search is case-insensitive by default"
                                        .to_string(),
                                ),
                                'n' => warnings.push(
                                    "note: ignored `-n`; every hit already carries file:line"
                                        .to_string(),
                                ),
                                _ => unreachable!("cluster filtered to known letters"),
                            }
                        }
                        continue;
                    }
                }
            }
            out.push(arg);
            continue;
        }
        if !saw_positional {
            saw_positional = true;
            if let Some(canonical) = query_looks_like_subcommand_typo(text) {
                // An exact non-search subcommand (`-w read REF`): the word
                // channel must never prefix a ref or non-query positional.
                if canonical == text
                    && !crate::cli_args::Commands::is_search_channel_name(text)
                {
                    nonsearch_subcommand = true;
                }
                if canonical != text
                    && (positional_recovery_is_safe(canonical, help_intent)
                        || vocab_recovery_is_read_only(canonical))
                {
                    warnings.push(format!(
                        "note: recovered `{text}` as `{canonical}`. Next time: asgrep {canonical} …"
                    ));
                    if canonical == "search" {
                        search_via_recovery = true;
                    }
                    out.push(std::ffi::OsString::from(canonical));
                    continue;
                }
                if canonical != text && !vocab_recovery_is_read_only(canonical) {
                    // Cross-surface vocabulary naming a mutating command is
                    // NEVER guessed into execution: teach the canonical form
                    // and let the (read-only) bare-search path answer instead
                    // of silently running e.g. `index` (which writes the
                    // workspace).
                    warnings.push(format!(
                        "note: `{text}` names the MCP tool `{canonical}`; the CLI canonical form is \
                         `asgrep {canonical} …` and mutating commands are never guessed into \
                         execution. If intended: asgrep {canonical} --help"
                    ));
                }
            }
        }
        if word_prefix
            && !word_prefixed
            && !nonsearch_subcommand
            && !pattern_seen
            && query_looks_like_subcommand_typo(text).is_none()
        {
            // grep `-w WORD` habit: the boundary mode is the `word:` channel.
            word_prefixed = true;
            out.push(std::ffi::OsString::from(format!("word:{text}")));
            continue;
        }
        if search_via_recovery && pattern_seen && !pattern_root_seen {
            // `--pattern P ROOT` (ast-grep batch habit): with the query
            // carried by flags the first bare positional is the ROOT.
            pattern_root_seen = true;
            out.push(std::ffi::OsString::from("--root"));
            out.push(arg);
            continue;
        }
        out.push(arg);
    }
    (out, warnings)
}

/// grep/rg long-flag vocabulary with an exact asgrep equivalent:
/// `(canonical long flag, the canonical flag takes a value)`.
fn habit_long_flag(folded: &str) -> Option<(&'static str, bool)> {
    match folded {
        "include" | "glob" => Some(("file-filter", true)),
        "max-results" | "max-count" => Some(("limit", true)),
        "type" => Some(("lang", true)),
        "word-regexp" => None,   // mode gap: taught by augment, never aliased
        "fixed-strings" => None, // mode gap
        _ => None,
    }
}

/// grep/rg long flags whose semantics are already the asgrep default.
fn habit_noop_long_flag(folded: &str) -> Option<&'static str> {
    match folded {
        "ignore-case" | "smart-case" => Some("search is case-insensitive by default"),
        "line-number" | "line-numbers" | "line-num" => Some("every hit already carries file:line"),
        _ => None,
    }
}

/// Short-flag letters recognised as grep/rg habit vocabulary (2026-10-22
/// intent corpus). Letters without a semantic equivalent are taught by
/// `augment_clap_usage_message` instead of being aliased.
const HABIT_SHORT_LETTERS: &[char] = &['l', 'C', 'g', 't', 'e', 'i', 'n', 'w', 'F', 'v', 'c', 'm'];

/// Habit short flags with no equivalent mode in fused hybrid search.
const MODE_GAP_SHORTS: &[char] = &['v', 'm'];

/// Human names for the mode gaps, used in clap-error teaching.
pub(crate) const MODE_GAP_NAMES: &[(char, &str)] = &[('v', "invert"), ('m', "per-file match cap")];

/// Exact cross-surface vocabulary targets that are safe to recover into
/// without the info-command gate: unambiguous read-only search/status verbs.
fn vocab_recovery_is_read_only(canonical: &str) -> bool {
    matches!(
        canonical,
        "search" | "keyword" | "semantic" | "status" | "read"
    )
}

pub(crate) fn query_looks_like_subcommand_typo(query: &str) -> Option<&'static str> {
    let q = query.trim();
    if q.is_empty() || q.contains(' ') {
        return None;
    }
    let lower = q.to_ascii_lowercase();
    // Plausible search tokens that sit near a command name at edit-distance 2.
    const SEARCH_SAFE: &[&str] = &[
        "static",
        "string",
        "struct",
        "switch",
        "symbol",
        "sample",
        "searchable",
    ];
    if SEARCH_SAFE.contains(&lower.as_str()) {
        return None;
    }
    const ALIASES: &[(&str, &str)] = &[
        ("capability", "capabilities"),
        ("robot_docs", "robot-docs"),
        ("robotdocs", "robot-docs"),
        ("robot-doc", "robot-docs"),
        ("docs", "robot-docs"),
        ("handbook", "robot-docs"),
    ];
    if let Some((_, canonical)) = ALIASES.iter().find(|(alias, _)| *alias == lower) {
        return Some(*canonical);
    }
    // Cross-surface vocabulary (2026-10-22 intent corpus): exact MCP/codemode
    // tool names and abbreviations an agent transfers to the CLI. Exact
    // matches only — never fuzzy, so a search query word that happens to be
    // close never reroutes. Mutating targets still pass the read-only gate
    // in `positional_recovery_is_safe`.
    const EXACT_VOCAB: &[(&str, &str)] = &[
        ("sem", "semantic"),
        ("keyword_search", "keyword"),
        ("semantic_search", "semantic"),
        ("code_search", "keyword"),
        ("code_read", "read"),
        ("index_status", "status"),
        ("grep", "search"),
        ("run", "search"),
        ("scan", "search"),
        ("ast", "search"),
        ("structural", "search"),
        ("syntax", "search"),
        ("tree", "search"),
        ("index_repo", "index"),
    ];
    if let Some((_, canonical)) = EXACT_VOCAB.iter().find(|(alias, _)| *alias == lower) {
        // Targets must name canonical commands (checked against `Commands::name`
        // via `all_names` so the table cannot drift from the clap inventory).
        debug_assert!(
            crate::cli_args::Commands::all_names().contains(canonical),
            "EXACT_VOCAB target must be a canonical command"
        );
        return Some(*canonical);
    }
    // Canonical names come from `Commands::all_names` (single source next to
    // `Commands::name`); the search aliases come from
    // `Commands::search_aliases` so the pair cannot drift.
    crate::cli_args::Commands::all_names()
        .iter()
        .chain(crate::cli_args::Commands::search_aliases().iter())
        .map(|command| (*command, edit_distance(&lower, command)))
        .min_by_key(|(_, distance)| *distance)
        .filter(|(command, distance)| {
            is_adjacent_transposition(&lower, command)
                || *distance <= 1
                || (*distance == 2
                    && lower.len().abs_diff(command.len()) <= 1
                    && lower.len().min(command.len()) >= 5)
        })
        .map(|(command, _)| command)
}

fn is_adjacent_transposition(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mismatches = left
        .bytes()
        .zip(right.bytes())
        .enumerate()
        .filter_map(|(index, (left, right))| (left != right).then_some((index, left, right)))
        .collect::<Vec<_>>();
    matches!(
        mismatches.as_slice(),
        [(first_index, first_left, first_right), (second_index, second_left, second_right)]
            if *second_index == *first_index + 1
                && first_left == second_right
                && first_right == second_left
    )
}

/// Byte Levenshtein distance shared by flag, command, and format suggestions.
pub(crate) fn edit_distance(left: &str, right: &str) -> usize {
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_byte) in left.bytes().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_byte) in right.bytes().enumerate() {
            current[right_index + 1] = if left_byte == right_byte {
                previous[right_index]
            } else {
                1 + previous[right_index]
                    .min(previous[right_index + 1])
                    .min(current[right_index])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}
