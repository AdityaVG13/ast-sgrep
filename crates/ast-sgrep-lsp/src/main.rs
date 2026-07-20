fn main() {
    let args: Vec<String> = std::env::args().collect(); if args.get(1).map(|s| s.as_str()) == Some("-V") || args.get(1).map(|s| s.as_str()) == Some("--version") {
        println!("asgrep-lsp {}", env!("CARGO_PKG_VERSION")); return;
    } if args.get(1).map(|s| s.as_str()) == Some("-h") || args.get(1).map(|s| s.as_str()) == Some("--help") {
        println!(
            "asgrep-lsp {}\nLSP search shell (library API). Full stdio server hollowed in this build.", env!("CARGO_PKG_VERSION")
        ); return;
    } eprintln!("asgrep-lsp: stdio server disabled (search library only)");
}
