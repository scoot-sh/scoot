//! The Linux shell: a Wayland compositor built on Smithay.
//!
//! It holds no layout of its own. Wayland says what windows exist, IPC and
//! keybindings say what should happen, and after either the compositor applies
//! whatever [`Arrangement`](scoot_core::Arrangement) the core produced.

mod activation;
mod alpha_modifier;
mod bind_budget;
mod child_reaper;
mod client_fds;
pub(crate) mod config;
mod content_type;
mod cursor;
mod decorations;
mod dispatch;
mod dmabuf;
mod drawn;
mod drm_syncobj;
mod ext_workspace;
mod fd_pressure;
mod floating;
mod foreign_toplevel;
mod foreign_toplevel_management;
mod fullscreen;
mod gamma_control;
mod handlers;
pub(crate) mod headless;
mod idle;
mod input;
mod input_method;
mod ipc;
mod keybindings;
mod keyboard_focus;
mod layer_shell;
mod nested;
mod nested_dispatch;
mod no_memory;
pub(crate) mod nofile;
mod output_clip;
mod output_identity;
mod output_management;
mod output_scale;
mod outputs;
mod popup;
mod popup_constraint;
mod popup_count;
mod popup_parent;
mod presentation_time;
mod reconnect;
mod relative_pointer;
mod reload;
pub(crate) mod render;
mod rounded;
mod screencopy;
mod screenshot;
mod selection;
mod session_env;
mod session_lock;
mod shell;
mod shm_pools;
mod sighup;
mod single_pixel_buffer;
mod state;
mod subsurface_depth;
mod tablet;
mod toplevel_cap;
mod toplevel_icon;
mod tty;
mod wayland_accept;
mod window_rules;
mod wl_buffers;
mod xwayland;

/// The harness the real-`wayland-client` test suites share. Not a module of
/// the compositor proper -- it exists only under `cfg(test)`.
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

use std::error::Error;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use crate::cli::CompositorOptions;
pub use state::State;

pub fn run(options: CompositorOptions) -> Result<(), Box<dyn Error>> {
    init_logging();

    // Before anything opens more than a handful of fds, and before the first
    // client connects: wayland-backend reads the limit when each client is
    // created to size its unclaimed-fd cap. Children get the original back
    // (see `nofile.rs`).
    nofile::raise();

    // Loaded before `State::new` so its `Config`/`Keybindings` can be handed
    // in directly rather than built as defaults and patched after the fact.
    // The only error this can return is an explicit `--config PATH` that
    // couldn't be read -- every other config problem already resolved
    // itself to defaults inside `load` (see `config`'s module doc).
    let loaded = config::load(options.config.as_deref())?;

    // `--nested` cannot honour a non-1.0 output scale: the host compositor
    // owns the scale of the window scoot is drawn in, so a scaled output
    // here would double-count it (render at 2x, then the host scales the
    // 2x-sized window again), and host-forwarded pointer coordinates arrive
    // in the host's logical space, which this compositor would then divide by
    // the configured scale a second time. Warn rather than silently accept a
    // broken session -- and rather than fail startup, which this project never
    // does over config (see `config.rs`'s module doc).
    let scale = if options.nested && loaded.scale != 1.0 {
        tracing::warn!(
            configured = loaded.scale,
            "output scaling is not supported under --nested (the host compositor owns \
             the window's scale); using 1.0"
        );
        1.0
    } else {
        loaded.scale
    };

    // Resolved before `State::new` for the same reason the config is: the
    // renderer is fixed for the process's life, and `State` carries it so a
    // later resize rebuilds the pipeline this session started with. `--tty`
    // is handled inside `resolve` (without the `gpu-scanout` feature it
    // warns and keeps pixman; with it, `--tty --renderer gles` selects the
    // scanout tier, falling back in `tty::init` when the device cannot
    // drive it), which is why it takes the flag rather than
    // reading it back out afterwards.
    let renderer = render::resolve(options.renderer, loaded.renderer, options.tty);

    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    let display: Display<State> = Display::new()?;
    let mut state = State::new(
        &mut event_loop,
        display,
        loaded.config,
        loaded.keybindings,
        loaded.appearance,
        scale,
        renderer,
    )?;

    // What `Request::Reload` re-reads: the same path startup used (the
    // explicit `--config` when one was given, else the resolved XDG default
    // -- even when no file existed there, so a file created later still
    // reloads). Plus the values a reload diffs against: the `[tty] gpu`
    // snapshot (never advanced), and the `[autostart]` snapshot seeding the
    // spawn delta (advanced by every unlocked reload; see `reload.rs`).
    // Both come off `loaded` before the autostart drain below moves it.
    state.config_path = config::startup_path(
        options.config.as_deref(),
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    );
    state.startup_gpu = loaded.gpu.clone();
    state.startup_autostart = loaded.autostart.clone();
    // Which windows float when they map: what a reload diffs against too.
    state.floating_rules = loaded.floating.clone();
    state.floating_modifier = loaded.floating_modifier;

    // `--tty` picks its own size from the connector's preferred mode (or
    // the `--mode` the user named) -- there's no host to negotiate a size
    // with the way `--nested` does, and no `--width`/`--height` to honor
    // the way `--headless` does -- so it has to run before
    // `headless::init`, which needs a concrete size to create the render
    // target at. Neither `--nested` nor plain `--headless` touch
    // `options.width`/`options.height` here.
    // `--tty` also names the output after its connector; the other two
    // backends have no connector, and keep the name clients have always
    // seen from them.
    //
    // The explicitly named DRM device, if any: `--gpu PATH` wins over
    // `[tty] gpu` when both name one, and an explicitly-set-but-empty path
    // in either is a hard startup error here rather than something the
    // session is asked to open (see `tty::resolve`). Resolved once so
    // both the `--tty` init below and the not-`--tty` warnings read the
    // same answer.
    let gpu = tty::resolve(options.gpu.as_deref(), loaded.gpu.as_deref())?;
    // One entry per output to create, in order (the first becomes the
    // primary): every connector `--tty` drives, each carrying the GPU scanout
    // renderer `tty::init` built alongside its `DrmCompositor` (which must be
    // built there, because it needs the renderer's formats) on its way to the
    // render target; or the one headless/nested output. See `ScanoutHandoff`.
    let heads: Vec<tty::StartupHead> = if options.tty {
        tty::init(state.loop_handle.clone(), &mut state, gpu, options.mode)?
    } else {
        // Not silently dropped the way `--width`/`--height` are under
        // `--tty`: those have a sensible reading on the backend that
        // ignores them (the mode wins), whereas `--gpu` or `--mode` on a
        // backend with no DRM device at all means the user believes they
        // are on `--tty` and is not.
        if options.gpu.is_some() {
            tracing::warn!("--gpu names the DRM device for --tty; ignoring it on this backend");
        }
        if loaded.gpu.is_some() {
            tracing::warn!("[tty] gpu names the DRM device for --tty; ignoring it on this backend");
        }
        if options.mode.is_some() {
            tracing::warn!("--mode picks the display mode for --tty; ignoring it on this backend");
        }
        vec![tty::StartupHead {
            width: options.width,
            height: options.height,
            name: headless::OUTPUT_NAME.to_owned(),
            identity: output_identity::OutputIdentity::named(headless::OUTPUT_NAME),
            scanout: render::ScanoutHandoff::default(),
        }]
    };
    let mut heads = heads.into_iter().enumerate();
    let Some((_, first)) = heads.next() else {
        // `tty::init` refuses a device with no head that builds, and the
        // other two backends always have their one output -- so this is
        // unreachable, and an error rather than a panic all the same.
        return Err("no output to create".into());
    };
    let output_name = first.name.clone();
    let primary = headless::init_named(
        &mut state,
        &first.name,
        first.width,
        first.height,
        first.scanout,
    )?;
    // The full connector identity (name plus EDID under `--tty`, name-only
    // elsewhere), replacing the name-only one `init_named` registered -- so
    // a later unplug files the right record (see `reconnect.rs`).
    state.note_output_identity(primary, first.identity);
    tty::attach(&mut state, 0, primary);
    // Every further `--tty` connector, side by side to the right of the one
    // before (`add_output_with` measures off the previous output's logical
    // geometry, so each output's own scale is respected). A connector whose
    // output cannot be created is a warning and a dark screen, never a
    // refused session -- under `--tty` that would be a lockout.
    for (index, head) in heads {
        match headless::add_output_with(
            &mut state,
            &head.name,
            head.width,
            head.height,
            head.scanout,
        ) {
            Ok(id) => {
                state.note_output_identity(id, head.identity);
                tty::attach(&mut state, index, id);
            }
            Err(error) => tracing::warn!(
                %error,
                connector = %head.name,
                "could not create an output for this connector"
            ),
        }
    }
    tty::retain_attached(&mut state);

    // The extra `--headless --outputs N` outputs, after the primary one --
    // `add_output` refuses to run before it, so this order is checked rather
    // than assumed. Warned about and ignored on the other two backends:
    // `--nested` presents one window in its host, and `--tty` already drives
    // every connected monitor, so a virtual output there would be a session
    // quietly different from the one that was asked for. Each is named after
    // the first (`headless-2`, `headless-3`, ...), which is what a client
    // sees as `wl_output.name`.
    if options.tty || options.nested {
        if options.outputs != 1 {
            tracing::warn!(
                "--outputs adds virtual outputs to --headless; ignoring it on this backend"
            );
        }
    } else {
        for index in 2..=options.outputs {
            let name = format!("{output_name}-{index}");
            headless::add_output(&mut state, &name, options.width, options.height)?;
        }
    }

    // Opt-in XWayland: after the outputs exist (X windows map into the
    // layout like any other, and the server is session-scoped, so it
    // starts once the session does) and before the `WAYLAND_DISPLAY` export
    // below, so the `DISPLAY` export joins it and both reach every spawned
    // child. `resolve` ORs the flag with the config key -- a flag can only
    // say yes -- and records the answer in `startup_xwayland` for reload to
    // diff against. A spawn failure is a loud log plus a Wayland-only
    // session, never a crash (see `xwayland/mod.rs`); without the Cargo feature
    // the knob warns once and does the same.
    let xwayland = xwayland::resolve(options.xwayland, loaded.xwayland);
    state.startup_xwayland = xwayland;
    if xwayland {
        #[cfg(feature = "xwayland")]
        match xwayland::start(state.loop_handle.clone(), &mut state) {
            Ok(display_number) => {
                tracing::info!(
                    display = display_number,
                    "XWayland server starting; X11 clients can connect once it is ready"
                );
            }
            Err(xwayland::StartError::Spawn(error)) => {
                tracing::error!(
                    %error,
                    "XWayland was requested but the server could not be started; \
                     continuing Wayland-only -- X11 applications will not run"
                );
            }
            Err(error @ xwayland::StartError::Insert(_)) => return Err(error.into()),
        }
        #[cfg(not(feature = "xwayland"))]
        tracing::warn!(
            "XWayland was requested (--xwayland or [xwayland] enabled) but this build \
             has no xwayland support (Cargo feature `xwayland`); continuing Wayland-only"
        );
    }

    // Order matters, and it's load-bearing, not incidental: `--nested`
    // connects to the *host* compositor via the caller's own WAYLAND_DISPLAY
    // (nested::init reads it via Connection::connect_to_env). That has to
    // happen before the set_var below replaces it with scoot's own socket
    // name for its own children -- reorder these and nested::init would
    // silently connect scoot to itself instead of its host, with no error
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
    // environment, so `scoot msg` works from inside the session too.
    //
    // Safety: `set_var` is unsound only if another thread could be reading
    // the environment concurrently. At this point in `run`, the event loop
    // hasn't started and no child process has been spawned yet, so this
    // process is still single-threaded and nothing else can observe a torn
    // read.
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
        // The X display, iff our server is believed live (see
        // `xwayland/mod.rs`): `None` leaves `DISPLAY` untouched, so a
        // host-provided value under `--nested` survives an XWayland-off
        // session -- no clobber.
        if let Some(display) = state.xdisplay {
            std::env::set_var(xwayland::DISPLAY_ENV, xwayland::display_value(display));
        }
        if let Some(path) = &state.ipc_path {
            std::env::set_var(scoot_ipc::SOCKET_ENV, path);
        }
        // Who this session is, for the programs inside it: the portal
        // backend lookup (`XDG_CURRENT_DESKTOP` against `scoot-portals.conf`)
        // and the session-type/desktop readers (Qt, xdg-autostart) that no
        // protocol carries. Settled once here -- see `session_env` for the
        // ownership rules (unconditional vs fill-the-vacuum) -- and again
        // per child in `State::spawn`, which applies the same `resolve`
        // explicitly rather than relying on this inheritance.
        //
        // Read before `resolve` (not inline in its arguments): `var` returns
        // owned strings, and `resolve` borrows the kept values, so the
        // owners have to live somewhere past the call.
        let current_desktop = std::env::var(session_env::CURRENT_DESKTOP).ok();
        let session_type = std::env::var(session_env::SESSION_TYPE).ok();
        let session_desktop = std::env::var(session_env::SESSION_DESKTOP).ok();
        let session_env = session_env::resolve(
            current_desktop.as_deref(),
            session_type.as_deref(),
            session_desktop.as_deref(),
        );
        std::env::set_var(session_env::CURRENT_DESKTOP, session_env.current_desktop);
        std::env::set_var(session_env::SESSION_TYPE, session_env.session_type);
        std::env::set_var(session_env::SESSION_DESKTOP, session_env.session_desktop);
        // The cursor theme this compositor resolved, so a client that loads a
        // theme *itself* picks the same one. `wp-cursor-shape-v1` covers the
        // clients that ask the compositor to draw for them (see
        // `cursor/theme.rs`), but GTK3 and anything predating that protocol
        // still load their own -- and would otherwise take the session's
        // default while scoot drew a different theme's shapes, which is the
        // inconsistency the protocol exists to remove.
        //
        // Exported unconditionally rather than only when a theme was found:
        // `XCURSOR_THEME` names the theme scoot *would* use, and a client
        // that has one installed where scoot found none should still use it
        // rather than fall back to something else again.
        std::env::set_var("XCURSOR_THEME", state.cursor.theme().name());
        std::env::set_var("XCURSOR_SIZE", state.cursor.theme().size().to_string());
    }

    // The SIGCHLD reaper, before the first spawn below: an exit between the
    // install and the first drain leaves no pending wakeup (its signal was
    // discarded or went elsewhere), and `install`'s synchronous drain is
    // what closes that race -- so the startup command must not run first.
    // Every later spawn (keybinding and IPC `spawn`) flows through the same
    // `State::spawn`, which is what tracks the pids this reaps.
    child_reaper::install(&event_loop.handle(), &mut state)?;

    // The SIGHUP reload trigger, next to the reaper: a HUP arriving before
    // this install keeps its default disposition and kills the session, so
    // this must not run after the first spawn either. Only the compositor
    // installs it -- `scootctl` keeps the default meaning of HUP.
    sighup::install(&event_loop.handle())?;

    // `[autostart]` first (the declared session baseline), then `--` (the
    // session script): both run through actions the session already knows
    // (`act` for the config entries, `spawn` for the one command), in the
    // same environment `--` has always had. The session starts unlocked, so
    // `act`'s lock gate passes here by construction; a non-`spawn` entry is
    // the user's choice (documented as such), and a config autostart plus a
    // script spawning the same bar yields two bars -- the same class as two
    // `spawn` binds, the user's composition to fix.
    for action in loaded.autostart {
        state.act(action);
    }

    if !options.command.is_empty() {
        let command = options.command.clone();
        state.spawn(&command);
    }

    tracing::info!(
        wayland = ?state.socket_name,
        ipc = ?state.ipc_path,
        renderer = %state.renderer,
        "scoot is up"
    );
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
/// With something actually queued, the cost is one `send()` per client that
/// has data waiting, once per wakeup rather than once per frame tick -- e.g.
/// a real pointer-motion flood measures as ~5.8x more wakeups on the client
/// side and a real, small, accepted compositor-side cost (~0.6% of one core
/// at realistic mouse rates); see `docs/roadmap/11-keystroke-flush.md` for the measured
/// numbers and methodology, not the paraphrase.
///
/// Errors are dropped deliberately, matching every other flush site here: the
/// only thing this can report is one client's socket refusing its bytes, which
/// is that client's problem -- wayland-backend already swallows the per-client
/// error internally (`flush(None)` does `let _ = client.flush()` per client and
/// returns `Ok`), so there is nothing actionable left to propagate.
fn post_dispatch(state: &mut State) {
    let _ = state.display_handle.flush_clients();
}

/// Colour only when a human is actually looking at a terminal.
///
/// `tracing_subscriber::fmt()` writes to stdout with ANSI on unconditionally
/// -- it never asks whether stdout is a terminal -- so every redirected or
/// piped capture used to arrive full of escape sequences. That is not
/// cosmetic here: this project's own hardware runbook captures logs with
/// `2>&1 | tee /tmp/fx/tty-auto.log` (`Asahi.md:278`), and a pipe is never a
/// terminal, so the canonical evidence path was the polluted one. It bit
/// `scripts/tty-tier-bench.sh`, which read the tier from
/// `scanout="gpu"` in a redirected log and silently matched nothing, because
/// what is really in the bytes is `scanout\x1b[0m\x1b[2m=\x1b[0m"gpu"`.
///
/// `stdout`, not `stderr`, because that is where `tracing_subscriber::fmt()`
/// writes by default; if the writer is ever pointed elsewhere, this has to
/// follow it or the gate silently tests the wrong file descriptor.
///
/// Guarded by `scripts/smoke-test.sh`, which redirects the compositor's
/// stdout to a file and asserts the file contains no escapes, rather than by
/// a unit test: in-process, the assertion could only say that the test
/// runner's own captured stdout is not a terminal, which measures nextest.
/// The regression is only observable from outside the process.
fn init_logging() {
    use std::io::IsTerminal;

    // The Smithay drm-compositor target is capped at `warn`: its only
    // `info!` at the pinned rev is the per-frame "failed to test cursor
    // plane state" on hardware whose kernel refuses the cursor TEST
    // (virtio-gpu does, every frame with damage), which would otherwise log
    // a line per frame indefinitely. Nothing else in that target logs at
    // `info`, so nothing else is lost -- and `RUST_LOG` still overrides all
    // of this per the usual `EnvFilter` rules when bringup needs the line
    // back (`RUST_LOG=info,smithay::backend::drm::compositor=info`).
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("info,smithay::backend::drm::compositor=warn")
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(std::io::stdout().is_terminal())
        .init();
}
