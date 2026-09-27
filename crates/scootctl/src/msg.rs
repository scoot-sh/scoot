//! `scootctl`: one request, one reply, for scripts and agents.

use std::error::Error;
use std::path::Path;

use scoot_ipc::{Client, Request, Response};

use crate::output;

pub fn run(request: &Request, out: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let mut client = Client::connect_default()?;
    // A subscription is not one request and one reply: after the
    // `Subscribed` answer the connection carries events until the session
    // ends (or drops the subscription), so this loops reading them rather
    // than returning after the first reply.
    if matches!(request, Request::Subscribe { .. }) {
        return subscribe(&mut client, request);
    }
    let response = client.request(request)?;
    // A human-readable heads-up on stderr, independent of what goes to
    // stdout below -- every non-error response's JSON goes to stdout the
    // same way regardless of which variant it is (see the catch-all arm),
    // so e.g. `scootctl key ... | jq .` keeps working and reflects what
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
                    // `output`): `scootctl screenshot | head -c0` exits 0.
                    output::print_line(&format!(
                        "{}x{}, {} bytes -> {}",
                        shot.width,
                        shot.height,
                        shot.png.len(),
                        path.display()
                    ))?;
                }
                // Straight to stdout, so `scootctl screenshot > shot.png` works.
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

/// Streams a subscription: prints the `Subscribed` answer, then one compact
/// JSON object per line per event, until the server closes the connection.
///
/// One object per line is the scriptable shape: `scootctl subscribe | jq`
/// stays a line protocol, and a `notify-send` wiring reads one line per
/// plug event. Exits 0 at a clean end of stream -- the session ended, or
/// the server dropped this subscription for not reading -- so re-run to
/// resubscribe (a supervisor's restart loop does that).
fn subscribe(client: &mut Client, request: &Request) -> Result<(), Box<dyn Error>> {
    let response = client.request(request)?;
    match &response {
        Response::Error { message } => return Err(message.clone().into()),
        Response::Subscribed { .. } => {}
        other => {
            return Err(format!("expected a subscription answer, got {other:?}").into());
        }
    }
    output::print_line(&serde_json::to_string(&response)?)?;
    loop {
        match client.next_message()? {
            Some(event) => output::print_line(&serde_json::to_string(&event)?)?,
            // A closed stdout is a quiet success, not an error (see
            // `output`): `scootctl subscribe | head -1` exits 0.
            None => return Ok(()),
        }
    }
}
