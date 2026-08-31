//! `pty remote-serve --stdio`: answer one request from a dialing peer.
//!
//! Only `--stdio` exists. Node also had `--socket <path>`, which is dropped
//! (docs/parity.md §12); a call without `--stdio` prints Node's usage block
//! and exits 1, which is what Node does for a missing `--socket` value too.
//!
//! node: src/cli.ts:1331-1339

use super::CliResult;

const USAGE: &str = "\
Usage: pty remote-serve --stdio

  Serve one remote request on stdin/stdout. Started per connection by
  `fabric expose pty-remote --exec -- pty remote-serve --stdio`.
";

pub fn run(args: &[String]) -> CliResult {
    if !args.iter().any(|a| a == "--stdio") {
        eprint!("{USAGE}");
        return Ok(1);
    }
    Ok(crate::remote::serve_stdio())
}
