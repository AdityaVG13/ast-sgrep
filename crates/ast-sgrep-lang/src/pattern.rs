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
pub(crate) mod bind;
pub(crate) mod calls;
pub(crate) mod classify;
pub(crate) mod collect;
pub(crate) mod core;
pub(crate) mod cs_statements;
pub(crate) mod directives;
pub(crate) mod dispatch;
pub(crate) mod gates;
pub(crate) mod general;
pub(crate) mod general_match;
pub(crate) mod ifs;
pub(crate) mod index;
pub(crate) mod literal;
pub(crate) mod meta_blocks;
pub(crate) mod native;
pub(crate) mod php;
pub(crate) mod preproc;
pub(crate) mod preprocess;
pub(crate) mod py_delete;
pub(crate) mod roots142;
pub(crate) mod statements;
pub(crate) mod structural;
pub(crate) mod templates;

pub use answerable::*;
pub(crate) use bind::*;
pub(crate) use calls::*;
pub use classify::*;
pub(crate) use collect::*;
pub use core::*;
pub(crate) use cs_statements::*;
pub(crate) use directives::*;
pub use dispatch::*;
pub use gates::*;
pub(crate) use general::*;
pub(crate) use general_match::*;
pub(crate) use ifs::*;
pub use index::*;
pub use literal::*;
pub(crate) use meta_blocks::*;
pub use native::*;
pub use php::*;
pub(crate) use preproc::*;
pub(crate) use preprocess::*;
pub(crate) use py_delete::*;
pub(crate) use roots142::*;
pub(crate) use statements::*;
pub(crate) use structural::*;
pub use templates::*;
