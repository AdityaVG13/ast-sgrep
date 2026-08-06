#![forbid(unsafe_code)]

pub mod backend;
pub mod server;
pub mod support;
pub mod types;
pub use backend::LspBackend;
pub use server::LspServer;
pub use support::path_to_file_uri;

pub mod uri {
    pub use crate::support::{
        canonicalize_workspace_root, file_uri_to_path, path_to_file_uri, uri_to_rel_path,
    };
}

pub mod convert {
    pub use crate::support::{
        call_hierarchy_endpoint, line_range, line_range_ext, location_value, workspace_symbol,
    };
}

pub mod symbols {
    pub use crate::support::{innermost_symbol, line_at_index};
}

pub mod text_edit {
    pub use crate::support::{apply_text_edit, extract_identifier_at, utf16_char_to_byte};
}

pub mod transport {
    pub use crate::support::{
        read_message, send_error, send_response, write_message, MAX_MESSAGE_BYTES,
    };
}

/// Settings surface used by testkit and LSP initialize options.
pub mod settings {
    pub use crate::support::AsgrepSettings;
}
