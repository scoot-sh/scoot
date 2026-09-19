//! `flexwm msg`: one request, one reply, for scripts and agents.

use std::error::Error;
use std::path::Path;

use flexwm_ipc::{Client, Request, Response};

use crate::output;

pub fn run(request: &Request, out: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let mut client = Client::connect_default()?;
    let response = client.request(request)?;
    // A human-readable heads-up on stderr, independent of what goes to
    // stdout below -- every non-error response's JSON goes to stdout the
    // same way regardless of which variant it is (see the catch-all arm),
    // so e.g. `flexwm msg key ... | jq .` keeps working and reflects what
    // actually happened whether or not there's a warning attached to it.
    // Advisory: if stderr itself is closed, the reply below still goes out.
    if let Response::Warning { message } = &response {
        output::warn(format_args!("warning: {message}"));
    }
    match response {
        Response::Screenshot(shot) => {
            match out {
                Some(path) => {
                    std::fs::write(path, &shot.png)?;
                    // A closed stdout is a quiet success, not an error (see
                    // `output`): `msg screenshot | head -c0` exits 0.
                    output::print_line(&format!(
                        "{}x{}, {} bytes -> {}",
                        shot.width,
                        shot.height,
                        shot.png.len(),
                        path.display()
                    ))?;
                }
                // Straight to stdout, so `flexwm msg screenshot > shot.png` works.
                None => output::write_bytes(&shot.png)?,
            }
            Ok(())
        }
        Response::Error { message } => Err(message.into()),
        other => {
            output::print_line(&serde_json::to_string_pretty(&other)?)?;
            Ok(())
        }
    }
}
