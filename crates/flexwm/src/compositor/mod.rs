//! The Linux shell: a Wayland compositor built on Smithay.
//!
//! It holds no layout of its own. Wayland says what windows exist, IPC and
//! keybindings say what should happen, and after either the compositor applies
//! whatever [`Arrangement`](flexwm_core::Arrangement) the core produced.

mod handlers;
mod headless;
mod input;
mod ipc;
mod screenshot;
mod shell;
mod state;

use std::error::Error;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use crate::cli::CompositorOptions;
pub use state::State;

pub fn run(options: CompositorOptions) -> Result<(), Box<dyn Error>> {
    init_logging();

    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    let display: Display<State> = Display::new()?;
    let mut state = State::new(&mut event_loop, display);

    headless::init(&mut state, options.width, options.height)?;
    ipc::init(&mut event_loop, &mut state, options.socket.clone())?;

    // Children reach both the compositor and its control socket through the
    // environment, so `flexwm msg` works from inside the session too.
    //
    // Safety: `set_var` is unsound only if another thread could be reading
    // the environment concurrently. At this point in `run`, the event loop
    // hasn't started and no child process has been spawned yet, so this
    // process is still single-threaded and nothing else can observe a torn
    // read.
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
        if let Some(path) = &state.ipc_path {
            std::env::set_var(flexwm_ipc::SOCKET_ENV, path);
        }
    }

    if !options.command.is_empty() {
        let command = options.command.clone();
        state.spawn(&command);
    }

    tracing::info!(wayland = ?state.socket_name, ipc = ?state.ipc_path, "flexwm is up");
    event_loop.run(None, &mut state, |_| {})?;
    Ok(())
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
