//! Return-section lane for function templates.

use super::*;
use crate::Language;
use tree_sitter::Node;

/// How a function template's return section constrains candidates (br-hs1,
/// probed 2026-10-27 against ast-grep 0.45.2, 3 stable rounds every cell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReturnLane {
    /// No spelled section. Body lane: the return must be ABSENT (the
    /// reference's return agreement). Body-less lane: wildcarded (br-f4w)
    /// — except the rust `;` form, where the semicolon makes the absent
    /// return REQUIRED (the strict signature reading).
    Unspecified { semicolon: bool },
    /// Metavariable return: presence required and the text binds — except
    /// go body-less, where absent results wildcard too.
    Metavar(String),
    /// Concrete return text: exact equality — with or without the `;`
    /// terminator (re-probed on the main fixture 2026-10-27: `-> u32;`
    /// answers the `-> u32` sigs; an earlier fixture-specific empty was a
    /// misread, not a quirk).
    Concrete { want: String, semicolon: bool },
    /// The br-osk no-parens PURE-NAME lane: every fn-shaped node answers,
    /// returns and generics wildcarded (probed: rust `fn $A` answers
    /// sigs ∪ items incl. `fn gen<T>`; go answers `[T any]` decls).
    Wildcard,
}

impl ReturnLane {
    pub(crate) fn unspecified() -> Self {
        ReturnLane::Unspecified { semicolon: false }
    }
}

pub(crate) fn return_lane_of(spec: Option<&ReturnSpec>, semicolon: bool) -> ReturnLane {
    match spec {
        None => ReturnLane::Unspecified { semicolon },
        Some(ReturnSpec::Metavar(name)) => ReturnLane::Metavar(name.clone()),
        Some(ReturnSpec::Concrete(text)) => ReturnLane::Concrete {
            want: text.clone(),
            semicolon,
        },
    }
}

/// Per-candidate return agreement. `bodyless_lane` selects the br-f4w
/// wildcard for section-less patterns and the go metavar wildcard.
pub(crate) fn return_section_matches(
    lang: Language,
    node: &Node,
    source: &str,
    lane: &ReturnLane,
    bodyless_lane: bool,
) -> bool {
    let declared = function_return_type_text(node, source);
    match lane {
        ReturnLane::Unspecified { semicolon } => {
            if *semicolon {
                declared.is_none()
            } else if bodyless_lane {
                true
            } else {
                declared.is_none()
            }
        }
        ReturnLane::Metavar(_) => {
            if bodyless_lane && lang == Language::Go {
                true
            } else {
                declared.is_some()
            }
        }
        ReturnLane::Concrete { want, .. } => declared == Some(want.as_str()),
        ReturnLane::Wildcard => true,
    }
}
