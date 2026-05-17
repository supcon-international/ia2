//! Thin wrapper that runs ironplc's LSP server over stdin/stdout. The PLC
//! server crate spawns this binary and bridges stdio to a WebSocket frame
//! stream the browser-side monaco-languageclient consumes.

fn main() {
    // env_logger respects RUST_LOG; logs go to stderr (so they don't collide
    // with LSP framed messages on stdout).
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .try_init();

    if let Err(e) = ironplc_cli::lsp::start() {
        eprintln!("ironplc lsp error: {e}");
        std::process::exit(1);
    }
}
