//! The Linux shell: a Wayland compositor built on Smithay.
//!
//! It holds no layout of its own. Wayland says what windows exist, IPC and
//! keybindings say what should happen, and after either the compositor applies
//! whatever [`Arrangement`](flexwm_core::Arrangement) the core produced.

mod config;
mod cursor;
mod decorations;
mod dispatch;
mod handlers;
mod headless;
mod input;
mod ipc;
mod keybindings;
mod nested;
mod nested_dispatch;
mod screenshot;
mod shell;
mod state;
mod tty;

#[cfg(test)]
mod tests;

use std::error::Error;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use crate::cli::CompositorOptions;
pub use state::State;

pub fn run(options: CompositorOptions) -> Result<(), Box<dyn Error>> {
    init_logging();

    // Loaded before `State::new` so its `Config`/`Keybindings` can be handed
    // in directly rather than built as defaults and patched after the fact.
    // The only error this can return is an explicit `--config PATH` that
    // couldn't be read -- every other config problem already resolved
    // itself to defaults inside `load` (see `config`'s module doc).
    let loaded = config::load(options.config.as_deref())?;

    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    let display: Display<State> = Display::new()?;
    let mut state = State::new(
        &mut event_loop,
        display,
        loaded.config,
        loaded.keybindings,
        loaded.appearance,
    );

    // `--tty` picks its own size from the connector's preferred mode --
    // there's no host to negotiate a size with the way `--nested` does, and
    // no `--width`/`--height` to honor the way `--headless` does -- so it
    // has to run before `headless::init`, which needs a concrete size to
    // create the render target at. Neither `--nested` nor plain
    // `--headless` touch `options.width`/`options.height` here.
    let (width, height) = if options.tty {
        tty::init(state.loop_handle.clone(), &mut state)?
    } else {
        (options.width, options.height)
    };
    headless::init(&mut state, width, height)?;

    // Order matters, and it's load-bearing, not incidental: `--nested`
    // connects to the *host* compositor via the caller's own WAYLAND_DISPLAY
    // (nested::init reads it via Connection::connect_to_env). That has to
    // happen before the set_var below replaces it with flexwm's own socket
    // name for its own children -- reorder these and nested::init would
    // silently connect flexwm to itself instead of its host, with no error
    // anywhere (this project has a documented history of exactly this shape
    // of bug: something quietly wrong because of missing flush or missing
    // ordering, not a crash). ipc::init has no such constraint; it's grouped
    // here only because it's the other one-time setup step.
    if options.nested {
        nested::init(
            state.loop_handle.clone(),
            &mut state,
            options.width,
            options.height,
        )?;
    }
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
    event_loop.run(None, &mut state, post_dispatch)?;
    Ok(())
}

/// Runs once after every dispatch cycle, whatever woke the loop up.
///
/// Wayland events queue into each client's own outgoing buffer and stay there
/// until something flushes it, so every wakeup that queues one has to be
/// followed by a flush or the client never hears about it. Doing that per call
/// site is a discipline to remember at every future one, and the one that was
/// missed is exactly the bug this exists for: `tty/mod.rs`'s libinput callback
/// (and `nested_dispatch.rs`'s host-forwarded equivalent) hands a real
/// keystroke to `input::key`, which queues a `wl_keyboard.key` for the focused
/// client and nothing else -- a keystroke does not mark the screen dirty, so
/// `render()` (which would have flushed at the end) early-returns on
/// `!needs_render`, and the frame timer has already dropped itself on a quiet
/// screen. The keystroke then sat in the buffer until some unrelated wakeup --
/// mouse motion, another client's traffic, an IPC request -- happened to flush
/// it, which was measured at seconds on real `--tty` hardware.
///
/// Here it is structural instead: anything queued during a wakeup goes out at
/// the end of that same wakeup, for every backend, whatever queued it.
///
/// The three flush sites that predate this one all stay, and none of them is
/// redundant work: each flushes *earlier within its own wakeup* than this does
/// -- `state.rs`'s wayland display source before the loop moves on to another
/// source, `headless.rs`'s `render()` after a frame's frame-callbacks, and
/// `ipc/connection.rs`'s `step()` before a reply's own round trip can race it
/// -- and a flush with nothing queued costs no syscall at all (see below), so
/// the overlap is free. What is now stale is only the *justification* written
/// at two of them ("nothing else flushes until..."): something else does now,
/// just later.
///
/// Cost, because this runs on every wakeup including the busiest ones (a
/// libinput motion event, an IPC message, a DRM vblank): with nothing queued,
/// `flush_clients` takes one mutex, walks the client list, and for each client
/// runs `BufferedSocket::flush`, whose write loop is `while written <
/// bytes.len()` over an empty buffer -- zero syscalls, just bookkeeping
/// (verified in wayland-backend 0.3.17's `rs/socket.rs` and
/// `rs/server_impl/handle.rs`). So it is O(clients) of pointer arithmetic per
/// wakeup, and -- the part that matters for idle CPU -- `EventLoop::run(None,
/// ..)` blocks in `dispatch` until a source is actually ready
/// (`loop_logic.rs`: `while !stop { dispatch(timeout)?; cb(data); }`), so an
/// idle compositor has no wakeups and therefore no flushes.
///
/// Errors are dropped deliberately, matching every other flush site here: the
/// only thing this can report is one client's socket refusing its bytes, which
/// is that client's problem -- wayland-backend already swallows the per-client
/// error internally (`flush(None)` does `let _ = client.flush()` per client and
/// returns `Ok`), so there is nothing actionable left to propagate.
fn post_dispatch(state: &mut State) {
    let _ = state.display_handle.flush_clients();
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
