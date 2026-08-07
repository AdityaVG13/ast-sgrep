#![forbid(unsafe_code)]

use ast_sgrep_lsp::server::LspServer;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 {
        match args[1].as_str() {
            "-h" | "--help" => {
                println!("asgrep-lsp {}\nLSP server for ast-sgrep hybrid code search (stdio).\n\nUSAGE:\n    asgrep-lsp [--stdio]\n\nFLAGS:\n    -h, --help       Print this help and exit\n    -V, --version    Print version and exit\n    --stdio          Speak LSP over stdio (default)", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-V" | "--version" => {
                println!("asgrep-lsp {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => {}
        }
    }
    let mut server = LspServer::new();
    match server.run() {
        Ok(()) => std::process::exit(server.process_exit_code()),
        Err(e) => {
            eprintln!("asgrep-lsp error: {e}");
            std::process::exit(1);
        }
    }
}
