//! Compact search-envelope rendering helpers.
//! Extracted from `lib.rs`. Envelope shaping only; session elision and tool
//! handlers stay in `lib`.

use super::PreviewMode;
use ast_sgrep_plugins::{
    CompactBudget, DetailLevel, OutputBudget, OutputFormat, format_response_with_budget,
    to_budgeted_compact_json,
};
use serde_json::Value;

pub(crate) fn render_compact_envelope(
    response: &ast_sgrep_core::SearchResponse,
    preview: PreviewMode,
    budget_tokens: Option<usize>,
) -> Value {
    match preview {
        PreviewMode::None => {
            let mut env = format_response_with_budget(
                response,
                OutputFormat::Compact,
                0,
                CompactBudget {
                    per_result_tokens: 0,
                    response_tokens: 0,
                },
            );
            blank_compact_snippets(&mut env);
            env
        }
        PreviewMode::Short => match budget_tokens {
            Some(max_tokens) => to_budgeted_compact_json(
                response,
                OutputBudget {
                    max_tokens,
                    default_detail: DetailLevel::Block,
                },
            ),
            None => format_response_with_budget(
                response,
                OutputFormat::Compact,
                0,
                CompactBudget::default(),
            ),
        },
        PreviewMode::Full => to_budgeted_compact_json(
            response,
            OutputBudget {
                max_tokens: budget_tokens.unwrap_or(8_192),
                default_detail: DetailLevel::Full,
            },
        ),
    }
}

fn blank_compact_snippets(envelope: &mut Value) {
    let Some(hits) = envelope.get_mut("h").and_then(Value::as_array_mut) else {
        return;
    };
    for hit in hits {
        let Some(row) = hit.as_array_mut() else {
            continue;
        };
        if row.len() > 4 {
            row[4] = Value::String(String::new());
        }
    }
}

/// FNV-1a over snippet bytes (v972). Content-keyed so an edited file re-sends.
pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
