//! Generates `docs/conformance.md`: one row per Node test file, mapped to the
//! Rust conformance test file(s) that port it (read from the `/// node:` doc
//! comments in `tests/*.rs`), or the reason it is not portable.
//!
//! Usage: `cargo run -p pty-conformance --bin conformance-map [--node <checkout>] [--out <path>]`.
//! Defaults: the checkout in `PTY_NODE_CHECKOUT` (or `/home/myobie/src/github.com/compoundingtech/pty`),
//! output to `docs/conformance.md` at the workspace root.

fn main() {
    if let Err(e) = conformance_map::run(std::env::args().skip(1).collect()) {
        eprintln!("conformance-map: {e}");
        std::process::exit(1);
    }
}

mod conformance_map {
    include!("../conformance_map_impl.rs");
}
