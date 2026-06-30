use ast_sgrep_lsp::server::LspServer;

fn main() {
    if let Err(e) = LspServer::new().run() {
        eprintln!("asgrep-lsp error: {e}");
        std::process::exit(1);
    }
}
