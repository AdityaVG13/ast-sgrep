//! Structural and literal pattern matching over tree-sitter ASTs.
//!
//! **Why this exists (vs shelling out to ast-grep):**
//! - Indexed hybrid search needs a fast, in-process structural channel.
//! - External `ast-grep` is excellent for full metavariable rules, but process
//!   spawn + JSON parse is too heavy for tight loops and offline agents.
//! - We implement the common ~80% of patterns natively (function/method/class
//!   decls and calls with `$NAME` / `$$$` holes). Exotic shapes are match-none
//!   or fail-closed in search; they are **not** silently shelled out to
//!   ast-grep (`DISC-pattern-native-subset`). Bench spawn is opt-in only.
//!
//! **Vocabulary used across the pattern modules:**
//! - *face*: one observable pattern shape plus its expected answer (a parity case).
//! - *lane*: one classification route (decl/call/general/...) a face travels.
//! - *census*: the per-file/per-language answer-count parity check; *census-loud*
//!   means a mismatch fails hard, never miscounts silently.
//! - *ingress/egress-loud*: refused loudly at classification (ingress) or at
//!   answer time (egress). *Probed/registered* shapes are pinned against the
//!   reference by the suite.

pub(crate) mod answerable;
pub(crate) mod arg_slots;
pub(crate) mod argument_helpers;
pub(crate) mod bind;
pub(crate) mod call_walks;
pub(crate) mod callee_resolve;
pub(crate) mod calls;
pub(crate) mod chain_links;
pub(crate) mod classify;
pub(crate) mod collect;
pub(crate) mod comments;
pub(crate) mod core;
pub(crate) mod csharp_expr_root;
pub(crate) mod cs_statements;
pub(crate) mod directives;
pub(crate) mod directives_cs_using;
pub(crate) mod directives_use_semi;
pub(crate) mod dispatch;
pub(crate) mod dollars;
pub(crate) mod dotted_chains;
pub(crate) mod gates;
pub(crate) mod general;
pub(crate) mod general_function_introspection;
pub(crate) mod general_if_body;
pub(crate) mod general_loop_head;
pub(crate) mod general_match;
pub(crate) mod general_nested_call;
pub(crate) mod general_placeholder_scan;
pub(crate) mod general_post_gates;
pub(crate) mod goto_dispatch;
pub(crate) mod ifs;
pub(crate) mod index;
pub(crate) mod java_sync;
pub(crate) mod lane_support;
pub(crate) mod literal;
pub(crate) mod member_chains;
pub(crate) mod meta_blocks;
pub(crate) mod native;
pub(crate) mod optional_chains;
pub(crate) mod php;
pub(crate) mod php_binary_split;
pub(crate) mod php_operand;
pub(crate) mod php_rhs_expr;
pub(crate) mod php_static_scope;
pub(crate) mod preproc;
pub(crate) mod preprocess;
pub(crate) mod py_delete;
pub(crate) mod return_lane;
pub(crate) mod return_sections;
pub(crate) mod roots142;
pub(crate) mod roots142_decl_match;
pub(crate) mod roots142_statements;
pub(crate) mod statements;
pub(crate) mod structural;
pub(crate) mod templates;
pub(crate) mod universal;

/// ONE trivia convention (H7): every pattern module sees the canonical
/// predicates through this glob (`use super::*`), so no lane keeps a
/// private copy that can drift.
pub(crate) use crate::trivia::*;

pub use answerable::*;
pub use arg_slots::*;
pub(crate) use argument_helpers::*;
pub(crate) use bind::*;
pub(crate) use call_walks::*;
pub(crate) use callee_resolve::*;
pub(crate) use calls::*;
pub(crate) use chain_links::*;
pub use classify::*;
pub(crate) use collect::*;
pub(crate) use comments::*;
pub use core::*;
pub(crate) use csharp_expr_root::*;
pub(crate) use cs_statements::*;
pub(crate) use directives::*;
pub(crate) use directives_cs_using::*;
pub(crate) use directives_use_semi::*;
pub use dispatch::*;
pub(crate) use dollars::*;
pub(crate) use dotted_chains::*;
pub use gates::*;
pub(crate) use general::*;
pub(crate) use general_function_introspection::*;
pub(crate) use general_if_body::*;
pub(crate) use general_loop_head::*;
pub(crate) use general_match::*;
pub(crate) use general_nested_call::*;
pub(crate) use general_placeholder_scan::*;
pub(crate) use general_post_gates::*;
pub(crate) use goto_dispatch::*;
pub(crate) use ifs::*;
pub use index::*;
pub(crate) use java_sync::*;
pub use literal::*;
pub(crate) use member_chains::*;
pub(crate) use meta_blocks::*;
pub use native::*;
pub(crate) use optional_chains::*;
pub use php::*;
pub(crate) use php_binary_split::*;
pub(crate) use php_operand::*;
pub(crate) use php_rhs_expr::*;
pub(crate) use php_static_scope::*;
pub(crate) use preproc::*;
pub(crate) use preprocess::*;
pub(crate) use py_delete::*;
pub(crate) use return_lane::*;
pub(crate) use return_sections::*;
pub(crate) use roots142::*;
pub(crate) use roots142_decl_match::*;
pub(crate) use roots142_statements::*;
pub(crate) use statements::*;
pub(crate) use structural::*;
pub use templates::*;
pub(crate) use universal::*;
