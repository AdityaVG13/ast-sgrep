//! ast-sgrep LSP server — Phase 6 full implementation.

pub mod backend;
pub mod server;
pub mod settings;
pub mod transport;
pub mod types;
pub mod uri;

pub use backend::LspBackend;
pub use server::LspServer;
pub use uri::path_to_file_uri;
