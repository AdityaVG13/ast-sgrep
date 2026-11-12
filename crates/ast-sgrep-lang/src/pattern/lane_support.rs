//! General-lane support predicate — MERGED into [`gates`](super::gates).
//!
//! The any-language admission now lives in
//! [`general_lane_supported`](super::gates::general_lane_supported), next to
//! the ingress gate that consults it. The triple admission (dispatch
//! routing / gates verdicts / this support predicate) is one list now.
//! This module stays as a documentation address so the merge history keeps
//! one place while the tree converges; it carries no items.
