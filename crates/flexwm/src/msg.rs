//! `flexwm msg`: one request, one reply, for scripts and agents.

use std::error::Error;
use std::io::Write;
use std::path::Path;

use flexwm_ipc::{Client, Request, Response};

pub fn run(request: &Request, out: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let mut client = Client::connect_default()?;
    match client.request(request)? {
        Response::Screenshot(shot) => {
            match out {
                Some(path) => {
                    std::fs::write(path, &shot.png)?;
                    println!(
                        "{}x{}, {} bytes -> {}",
                        shot.width,
                        shot.height,
                        shot.png.len(),
                        path.display()
                    );
                }
                // Straight to stdout, so `flexwm msg screenshot > shot.png` works.
                None => std::io::stdout().write_all(&shot.png)?,
            }
            Ok(())
        }
        Response::Warning { message } => {
            eprintln!("warning: {message}");
            Ok(())
        }
        Response::Error { message } => Err(message.into()),
        other => {
            println!("{}", serde_json::to_string_pretty(&other)?);
            Ok(())
        }
    }
}
