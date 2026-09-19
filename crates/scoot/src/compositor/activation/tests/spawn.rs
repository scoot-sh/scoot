//! The token a process scoot spawned itself carries in `$XDG_ACTIVATION_TOKEN`.
//!
//! Whoever starts a process conventionally hands it an activation token, so
//! the child can activate its own window when it finally maps one.
//! `State::spawn` (every keybinding and IPC `spawn`) used to set
//! `WAYLAND_DISPLAY` and `SCOOT_SOCKET` but no token, so an app scoot
//! itself started could not ask to be focused the way one started by a
//! launcher client can -- invisible in practice, because mapping focuses the
//! new window itself, until a slow cold start lets focus move elsewhere
//! first.
//!
//! These tests drive the real `State::spawn` with a real child process: a
//! `sh` probe that writes whatever `$XDG_ACTIVATION_TOKEN` it saw to a file,
//! which is the closest honest fixture to a toolkit reading the variable --
//! it proves the variable is really in the child's environment, not just set
//! on a `Command` nobody executed. The token string that comes back over the
//! file is then redeemed through the real `request_activation`, so the focus
//! assertions exercise the same lock gate, single-use removal and
//! `clicked_layer` spend a toolkit's own `activate` would.
//!
//! Like every other live-[`State`] test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket. They
//! also spawn a real `sh`, which the dev VM and any Unix test host have.

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use scoot_core::Action;
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};

use super::*;
use crate::compositor::test_support::wait_for;

/// A live compositor with one output and one connected client mapping two
/// windows, the shape [`drive`] builds. The client takes no steps.
type Fixture = Harness<(), Ack>;

/// What the probe child writes when `State::spawn` minted it no token.
///
/// A fixed sentinel rather than "empty": it distinguishes "the child ran and
/// the variable was unset" from "the child never ran" (no file at all, which
/// [`read_probe`] reports as a timeout instead). `State::spawn` removes any
/// inherited value first, so an unset variable here always means no token
/// was minted rather than a clean environment elsewhere.
const NO_TOKEN: &str = "SPAWN_MINTED_NO_TOKEN";

/// Spawns a real child through the real [`State::spawn`] that writes whatever
/// `$XDG_ACTIVATION_TOKEN` it saw to a fresh file, and hands back that file's
/// path.
///
/// The child is `sh -c '... "$1"'` with the path as `$1`, so no filename
/// ever goes through shell quoting.
fn spawn_probe(fixture: &mut Fixture) -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "scoot-spawn-token-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        "printf '%s' \"${XDG_ACTIVATION_TOKEN:-SPAWN_MINTED_NO_TOKEN}\" > \"$1\"".to_string(),
        "sh".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    fixture.state.spawn(&command);
    path
}

/// Waits for the probe child's file to appear with content, reads it, and
/// removes the file.
///
/// Content, not mere existence: the shell creates the file before `printf`
/// writes to it, so an early read would see an empty file and misreport a
/// minted token as missing. Every outcome the tests produce is non-empty (a
/// 32-character token or [`NO_TOKEN`]).
fn read_probe(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(body) = std::fs::read_to_string(path)
            && !body.is_empty()
        {
            let _ = std::fs::remove_file(path);
            return body;
        }
        assert!(
            Instant::now() < deadline,
            "the spawned child never wrote its token file: {path:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The token data the compositor holds for the string a probe child saw.
///
/// Fails the test when the child saw [`NO_TOKEN`] or a string the table does
/// not know: redeeming an unknown string directly would still move focus
/// (`request_activation` trusts the data it is handed), so asserting focus
/// alone would pass against an implementation that minted nothing.
fn data_for_child_token(
    fixture: &Fixture,
    token_string: &str,
) -> (XdgActivationToken, XdgActivationTokenData) {
    assert_ne!(
        token_string, NO_TOKEN,
        "State::spawn gave its child no XDG_ACTIVATION_TOKEN"
    );
    let token = XdgActivationToken::from(token_string.to_string());
    let data = fixture
        .state
        .xdg_activation
        .data_for_token(&token)
        .expect("the child's token is unknown to the compositor")
        .clone();
    (token, data)
}

#[test]
fn a_spawned_child_sees_its_activation_token_in_its_environment() {
    let (mut fixture, _run) = drive(false, 0, Claim::RealKeyPress);
    let path = spawn_probe(&mut fixture);
    let token_string = read_probe(&path);
    let _ = data_for_child_token(&fixture, &token_string);
}

#[test]
fn a_spawned_childs_token_moves_focus_after_focus_moved_elsewhere() {
    // The slow-cold-start shape the ticket is about: at spawn time focus is
    // on the first window, the user moves on to the second while the child
    // starts, and only then does the child map and redeem. The child itself
    // (`sh`) maps nothing, so any focus change below comes from the token,
    // not from mapping.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let first = window_of(&fixture, run.first_surface);
    let second = fixture
        .state
        .focus
        .expect("a focused window once two are mapped");
    assert_ne!(Some(first), Some(second));
    fixture.state.act(Action::FocusWindowId(first));
    assert_eq!(fixture.state.focus, Some(first));

    let path = spawn_probe(&mut fixture);

    fixture.state.act(Action::FocusWindowId(second));
    assert_eq!(
        fixture.state.focus,
        Some(second),
        "moving focus after the spawn did not stick, so redeeming would prove nothing"
    );

    let token_string = read_probe(&path);
    let (token, data) = data_for_child_token(&fixture, &token_string);
    let surface = surface_of(&fixture, run.first_surface);
    fixture.state.request_activation(token, data, surface);
    assert_eq!(
        fixture.state.focus,
        Some(first),
        "redeeming the spawned child's token did not move focus"
    );
}

#[test]
fn a_spawned_childs_token_cannot_be_spent_twice() {
    // The table's single-use rule, confirmed for a compositor-minted token:
    // `request_activation` removes the token whether or not it honored it.
    let (mut fixture, run) = drive(false, 0, Claim::RealKeyPress);
    let first = window_of(&fixture, run.first_surface);
    let second = fixture
        .state
        .focus
        .expect("a focused window once two are mapped");

    let path = spawn_probe(&mut fixture);
    let token_string = read_probe(&path);
    let (token, data) = data_for_child_token(&fixture, &token_string);
    let surface = surface_of(&fixture, run.first_surface);
    fixture.state.request_activation(token, data, surface);
    assert_eq!(fixture.state.focus, Some(first));
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
        0,
        "the redeemed spawn token is still outstanding"
    );

    // Moved on again, and the same string offered a second time buys
    // nothing: the table no longer knows it, so Smithay's dispatch -- which
    // only calls `request_activation` for a token it still holds -- never
    // reaches the handler. (Calling the handler directly with the stale
    // clone would bypass that lookup, a shape no protocol client can
    // produce: clients send only the string, and the data comes from the
    // table. What the direct call pins is the removal that lookup depends
    // on.)
    fixture.state.act(Action::FocusWindowId(second));
    let replay = XdgActivationToken::from(token_string);
    assert!(
        fixture
            .state
            .xdg_activation
            .data_for_token(&replay)
            .is_none(),
        "a redeemed spawn token is still redeemable"
    );
    assert_eq!(
        fixture.state.focus,
        Some(second),
        "a redeemed spawn token moved focus a second time"
    );
}

#[test]
fn spawn_tokens_share_the_cap_and_a_full_table_mints_nothing() {
    // The bound decision: spawn tokens live in the same table under the same
    // 64-token cap, and a full table means this child gets no token -- never
    // an eviction of someone else's live token. (This passes even without the
    // implementation, by design: it pins what `spawn` must *not* do, the way
    // the IPC layout-action boundary test pins its own not-spending side.)
    let (mut fixture, _run) = drive(false, 0, Claim::RealKeyPress);
    let mut live = Vec::with_capacity(MAX_TOKENS);
    for _ in 0..MAX_TOKENS {
        let (token, _) = fixture
            .state
            .xdg_activation
            .create_external_token(None::<XdgActivationTokenData>);
        live.push(token.clone());
    }
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
        MAX_TOKENS,
        "the test setup did not fill the token table"
    );

    let path = spawn_probe(&mut fixture);
    assert_eq!(
        read_probe(&path),
        NO_TOKEN,
        "spawn minted a token past a full table"
    );
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
        MAX_TOKENS,
        "a spawn past a full table grew or shrank it"
    );
    for token in &live {
        assert!(
            fixture.state.xdg_activation.data_for_token(token).is_some(),
            "a spawn past a full table evicted a live token"
        );
    }
}

#[test]
fn expired_tokens_are_swept_before_a_spawn_mints() {
    // The other half of the bound decision: a table full of *expired* tokens
    // is not full. The spawn path sweeps first, the way `token_created` does,
    // so a burst of long-dead spawns cannot wedge every spawn after it -- and
    // a leaked child that never redeems stops costing a slot after 30s.
    let (mut fixture, _run) = drive(false, 0, Claim::RealKeyPress);
    for _ in 0..MAX_TOKENS {
        let stale = XdgActivationTokenData {
            timestamp: Instant::now() - (TOKEN_LIFETIME + Duration::from_secs(1)),
            ..XdgActivationTokenData::default()
        };
        fixture.state.xdg_activation.create_external_token(stale);
    }
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
        MAX_TOKENS,
        "the test setup did not fill the token table"
    );

    let path = spawn_probe(&mut fixture);
    let token_string = read_probe(&path);
    let _ = data_for_child_token(&fixture, &token_string);
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
        1,
        "expired tokens were not swept before the spawn minted"
    );
}

#[test]
fn a_spawn_that_never_starts_leaves_no_token_behind() {
    // Minting happens before `Command::spawn`, so a child that never starts
    // (missing binary, overlong name, and the empty command that never gets
    // as far as minting) must not leave its token sitting until the sweep.
    // (Passes without the implementation too -- nothing is ever minted there
    // -- and guards the removal arm once it exists: commenting that arm out
    // fails the first two assertions.)
    let (mut fixture, _run) = drive(false, 0, Claim::RealKeyPress);
    fixture
        .state
        .spawn(&["scoot-test-no-such-program".to_string()]);
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
        0,
        "a failed spawn left its token in the table"
    );
    fixture.state.spawn(&["x".repeat(4096)]);
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
        0,
        "a failed spawn left its token in the table"
    );
    fixture.state.spawn(&[]);
    assert_eq!(
        fixture.state.xdg_activation.tokens().count(),
        0,
        "an empty spawn command minted a token for no child"
    );
}

// -------------------------------------------------------------------------
// Locked between spawn and redeem
// -------------------------------------------------------------------------

/// The framebuffer the headless lock-test backend renders into. Nothing reads
/// a pixel; the backend exists so a session lock can be confirmed, which
/// takes a drawn frame and [`Harness::bare`] never produces.
const CANVAS: i32 = 200;

/// The lock end, the shape `keyboard.rs`'s own locker client uses: takes the
/// session lock the way a lock screen does and maps no lock surface. The one
/// `()` this client ever receives means "unlock now".
#[derive(Default)]
struct LockerClient {
    manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    locked: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for LockerClient {
    fn event(
        client: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        if interface == ext_session_lock_manager_v1::ExtSessionLockManagerV1::interface().name {
            client.manager = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for LockerClient {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_v1::Event::Locked = event {
            client.locked = true;
        }
    }
}

wayland_client::delegate_noop!(LockerClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);

fn run_locker(stream: UnixStream, steps: Receiver<()>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = LockerClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let manager = client
        .manager
        .clone()
        .ok_or("no ext_session_lock_manager_v1 -- the global is missing")?;
    // Held so the session stays locked: dropping the lock object ends the
    // lock, which is exactly what must not happen until the test unlocks.
    let lock = manager.lock(&qh, ());
    wait_for(&mut queue, &mut client, "the session lock", |client| {
        client.locked.then_some(())
    })?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    acks.send(Ack::Locked).map_err(|e| e.to_string())?;

    steps.recv().map_err(|e| e.to_string())?;
    lock.unlock_and_destroy();
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    acks.send(Ack::Unlocked).map_err(|e| e.to_string())?;
    while steps.recv().is_ok() {}
    Ok(())
}

#[test]
fn a_spawned_childs_token_is_refused_while_locked() {
    // The ticket's edge case: the session locks between spawn and redeem. The
    // refusal must hold for a compositor-minted token exactly as for a
    // launcher's, and unlocking must not have spent anything -- a fresh
    // redeem afterwards still works, which also proves the refusal was the
    // lock and not a bad token.
    //
    // A headless backend, not [`drive`]'s bare compositor: the lock is only
    // confirmed (and the `Locked` event only sent) once a blanked frame has
    // been drawn, which nothing renders without a backend.
    let mut fixture = Harness::headless(Appearance::default(), CANVAS);
    fixture.spawn(|stream, _steps, acks| {
        map_two_and_activate_first(stream, false, 0, Claim::RealKeyPress, acks)
    });
    let Ack::Mapped = fixture.wait_for_ack(0) else {
        panic!("the client reported it was done before it reported being mapped");
    };
    press_a_key(&mut fixture);
    let _ = fixture.state.display_handle.flush_clients();
    let Ack::Done(run) = fixture.wait_for_ack(0) else {
        panic!("the client reported being mapped twice");
    };
    let first = window_of(&fixture, run.first_surface);
    let focus_before = fixture.state.focus;

    let path = spawn_probe(&mut fixture);
    fixture.spawn(run_locker);
    let Ack::Locked = fixture.wait_for_ack(1) else {
        panic!("the locker client never took the session lock");
    };
    fixture.settle();
    assert!(fixture.state.session_lock.is_locked());

    let token_string = read_probe(&path);
    let (token, data) = data_for_child_token(&fixture, &token_string);
    let surface = surface_of(&fixture, run.first_surface);
    fixture
        .state
        .request_activation(token, data, surface.clone());
    assert_eq!(
        fixture.state.focus, focus_before,
        "a locked session's focus was moved by a spawned child's token"
    );

    let Ack::Unlocked = fixture.run_on(1, ()) else {
        panic!("the locker client never unlocked the session");
    };
    assert!(!fixture.state.session_lock.is_locked());
    assert_eq!(
        fixture.state.focus, focus_before,
        "unlocking moved window focus"
    );

    let path = spawn_probe(&mut fixture);
    let token_string = read_probe(&path);
    let (token, data) = data_for_child_token(&fixture, &token_string);
    fixture.state.request_activation(token, data, surface);
    assert_eq!(
        fixture.state.focus,
        Some(first),
        "the spawn token stopped working after the lock cycle"
    );
}

/// The `sh` the fd probe below runs in the child.
///
/// Three sections, one file, written under a temporary name and `mv`d into
/// place so [`read_probe`] -- which returns on the first non-empty read --
/// can never see a half-written listing:
///
/// - `fd <number> <target>` per open fd, from `readlink`, so every fd is
///   identified by what it points at rather than by its number.
/// - the whole of `fdinfo`, `grep -H ''`-prefixed with the file each line
///   came from, which is where the eventfd marker's identity lives (see the
///   test's own comment on `eventfd-count`).
///
/// `/proc/$$/fd`, never `/proc/self/fd`: `self` resolves against whatever
/// process opens the path, so `readlink` and `grep` would each report their
/// *own* table. `$$` stays the shell's pid inside a command substitution,
/// which is the process that did the inheriting.
const FD_PROBE: &str = r#"
for f in /proc/$$/fd/*; do
    printf 'fd %s %s\n' "${f##*/}" "$(readlink "$f")"
done > "${1}.tmp"
grep -H '' /proc/$$/fdinfo/* >>"${1}.tmp" 2>/dev/null
mv "${1}.tmp" "$1"
"#;

/// The count in an `eventfd-count: <hex>` line, from the compositor's own
/// `/proc/self/fdinfo/<n>` or from the child's `grep -H ''` dump (which
/// prefixes every line with the file it came from, so the key is mid-line
/// there). The kernel prints it as `%16llx` -- space-padded lowercase hex --
/// which is parsed rather than string-matched, so padding or width changing
/// cannot silently turn the child-side check into a no-op.
fn eventfd_count(line: &str) -> Option<u64> {
    let (_, count) = line.split_once("eventfd-count:")?;
    u64::from_str_radix(count.trim(), 16).ok()
}

/// A `State::spawn` child inherits no close-on-exec compositor fd.
///
/// `--tty` used to pass `OFlags::CLOEXEC` when opening the DRM fd through
/// the session, but the pinned Smithay's `LibSeatSession::open` discards its
/// flags argument entirely (`_flags`, `backend/session/libseat.rs` at the
/// pinned rev), so the flag was dead code and has been removed. The
/// guarantee it appeared to provide still holds, and close-on-exec is the
/// whole of it: every seatd-obtained fd carries the bit from libseat's own
/// receive path (measured live 2026-09-13: the DRM and input fds all report
/// it), and `execve` closes such fds in the child. This test pins the
/// `spawn` side of that chain with the real `State::spawn` and a real child:
/// marker fds with close-on-exec set must be absent from the child's fd
/// table. The markers cover every fd shape the compositor holds across a
/// spawn, not just plain files: the seatd-fd-shaped file open, a connected
/// socket pair (the listener/accepted-stream shape -- every one is a std
/// socket), a `try_clone` of one end (the parked-screenshot and `PendingIdle`
/// clone shape), and an `eventfd` (the calloop channel-ping shape behind the
/// screenshot completion channel and the session notifier). A second marker
/// deliberately *without* close-on-exec is the positive control: it must be
/// PRESENT, which proves the probe observes real inheritance -- and proves
/// the bit is load-bearing, because `State::spawn` provably inherits any fd
/// lacking it.
/// (First written asserting both markers absent, it failed exactly on the
/// plain marker -- the child's table held it -- which is both the fail-first
/// record and the reason the control asserts presence, permanently. The
/// socket/clone/eventfd markers were each proven sensitive the same way, by
/// clearing the bit on one marker and watching exactly that marker appear in
/// the child.) If Rust std ever starts closing every fd at spawn, the control
/// goes red: revisit then, the guarantee will have moved.
///
/// **Every check here is by fd *identity*, never by fd number**, and that is
/// load-bearing rather than fastidious. The number-based version of this test
/// went red on an unrelated docs-only PR (CI run 35460052576, `cargo test
/// --workspace`): a number says nothing about *which* open file description
/// it names, in either direction. The child's own fds (the shell's redirect,
/// the listing process's directory fd) take the lowest free numbers, and
/// after `execve` those are exactly the numbers the close-on-exec fds just
/// vacated -- so a marker at fd 3 reappears as the child's own fd 3 and reads
/// as a leak. The other direction is the same coin: under `cargo test` every
/// test shares one process, so a neighbour's fd (`gamma_control/tests.rs`
/// opens a deliberately plain `libc::pipe`, and any test that closes one
/// frees its number) can be the thing sitting on a marker's number. nextest,
/// running each test in its own process, structurally cannot see either --
/// which is why this has to be right rather than merely green there.
/// So: each file marker gets a path nothing else in this process opens, and
/// is matched by that path; the sockets are matched by the `socket:[inode]`
/// their target carries, taken from `fstat` on this side; the eventfd, whose
/// target is the shape-only `anon_inode:[eventfd]`, is matched by a
/// distinctive starting count read back out of `fdinfo`. An open fd's path
/// and inode are exclusively its own for as long as it stays open, which the
/// liveness check at the end of the test is what guarantees.
#[test]
fn a_spawned_child_inherits_no_close_on_exec_fd() {
    let mut fixture: Harness<(), ()> = Harness::bare(Appearance::default());

    /// A marker file, held open across the spawn and removed when the test
    /// ends (on the panicking path too, which is what `Drop` buys over a
    /// tidy-up line at the bottom).
    struct FileMarker {
        /// Canonical, and what the child's `readlink` reports for the fd.
        path: PathBuf,
        fd: OwnedFd,
    }

    impl Drop for FileMarker {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Opens a fresh file nothing else in this process opens, with
    /// close-on-exec exactly as asked: the `cloexec` marker is the
    /// seatd-obtained-fd shape, the plain one the worst case.
    ///
    /// A unique path, not `/dev/null` (what these markers used to be),
    /// because the path *is* the identity here and half a process points at
    /// `/dev/null`. `libc::open`, not `std::fs::File`, because std sets
    /// close-on-exec unconditionally and the plain marker needs it absent.
    fn open_marker(cloexec: bool) -> FileMarker {
        static NEXT_MARKER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "scoot-spawn-fd-marker-{}-{}",
            std::process::id(),
            NEXT_MARKER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, b"").expect("a writable temp dir for the marker file");
        // Canonicalized because `/proc/<pid>/fd/<n>` readlinks to the real
        // path: a symlinked `$TMPDIR` would otherwise make every comparison
        // below miss, and the absence checks would pass vacuously.
        let path = std::fs::canonicalize(&path).expect("the marker file just written");
        let c_path = CString::new(path.as_os_str().as_bytes()).expect("a temp path with no NUL");
        let flags = if cloexec {
            libc::O_RDONLY | libc::O_CLOEXEC
        } else {
            libc::O_RDONLY
        };
        let fd = unsafe { libc::open(c_path.as_ptr(), flags) };
        assert!(fd >= 0, "could not open the marker file {}", path.display());
        // SAFETY: `open` just returned this fd; it is open and owned.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        FileMarker { path, fd }
    }
    let cloexec_marker = open_marker(true);
    let plain_marker = open_marker(false);
    // The socket shapes: a connected pair (what every listener, accepted
    // stream and parked entry is made of) plus a `try_clone` of one end
    // (the parked-screenshot / `PendingIdle` clone). The eventfd is the
    // calloop channel-ping shape (screenshot completions, session events).
    // `libc::eventfd`, not a channel constructor, because the pin is about
    // the fd flag, and this names the exact flag at creation.
    let (sock_a, sock_b) = UnixStream::pair().expect("a socket pair");
    let sock_clone = sock_a.try_clone().expect("a cloned socket");
    // The eventfd's starting count is its identity: `readlink` reports every
    // eventfd as `anon_inode:[eventfd]`, naming no instance, but `fdinfo`
    // reports the count, and no other eventfd in this process carries a
    // value like this one (calloop's pings start at 0 and are read back to 0
    // after every wake). The pid is in it so two scoot test processes
    // sharing a machine stay distinguishable in any captured output; only
    // this process's own table can ever reach the child.
    let event_count = 0xEF00_0000 | (std::process::id() & 0x00FF_FFFF);
    let event = unsafe { libc::eventfd(event_count, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    assert!(event >= 0, "could not create the eventfd marker");
    // SAFETY: `eventfd` just returned this fd; it is open and owned.
    let event = unsafe { OwnedFd::from_raw_fd(event) };
    // Fail loud, not weak: if the platform forced close-on-exec onto the
    // plain marker, the presence control below would go red on a false
    // premise instead of silently pinning a weaker claim.
    let flag_of = |fd: i32| unsafe { libc::fcntl(fd, libc::F_GETFD) };
    assert_ne!(
        flag_of(cloexec_marker.fd.as_raw_fd()) & libc::FD_CLOEXEC,
        0,
        "the close-on-exec marker is missing close-on-exec"
    );
    assert_eq!(
        flag_of(plain_marker.fd.as_raw_fd()) & libc::FD_CLOEXEC,
        0,
        "the worst-case marker unexpectedly carries close-on-exec"
    );
    for (what, fd) in [
        ("socket pair end", sock_a.as_raw_fd()),
        ("socket pair end", sock_b.as_raw_fd()),
        ("cloned socket", sock_clone.as_raw_fd()),
        ("eventfd", event.as_raw_fd()),
    ] {
        assert_ne!(
            flag_of(fd) & libc::FD_CLOEXEC,
            0,
            "the {what} marker is missing close-on-exec"
        );
    }
    // The same premise guard for the eventfd's identity: if its count were
    // not visible, or not the value asked for, the child-side check could
    // not see it either and would pass against a real leak.
    let event_fdinfo = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", event.as_raw_fd()))
        .expect("this platform has /proc/self/fdinfo");
    assert_eq!(
        event_fdinfo.lines().find_map(eventfd_count),
        Some(u64::from(event_count)),
        "the eventfd marker does not report the count that identifies it, so \
         the child-side check below would prove nothing: {event_fdinfo:?}"
    );
    // A socket's target is `socket:[<inode>]`, and the inode is `fstat`'s --
    // unique among live sockets, and unreusable while the fd stays open.
    // `sock_clone` shares `sock_a`'s description and so its inode: one
    // identity covers both, and a leak of either surfaces as that inode (the
    // bit is per-fd-slot, so both are still checked for it above).
    let socket_target = |what: &str, socket: &UnixStream| {
        let stat = rustix::fs::fstat(socket).unwrap_or_else(|e| panic!("fstat on the {what}: {e}"));
        format!("socket:[{}]", stat.st_ino)
    };
    let pair_target = socket_target("socket pair end", &sock_a);
    let other_target = socket_target("other socket pair end", &sock_b);
    assert_eq!(
        pair_target,
        socket_target("cloned socket", &sock_clone),
        "a try_clone of a socket reports a different inode than its original"
    );
    assert_ne!(
        pair_target, other_target,
        "the two ends of a socket pair report one inode, so one absence check covers both"
    );

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "scoot-spawn-fds-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        FD_PROBE.to_string(),
        "sh".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    fixture.state.spawn(&command);
    let listing = read_probe(&path);
    // Every `fd <number> <target>` line, as (number, target). A target can be
    // empty: the shell's own glob directory fd is in the expansion and gone
    // again by the time `readlink` runs on it.
    let child_fds: Vec<(&str, &str)> = listing
        .lines()
        .filter_map(|line| line.strip_prefix("fd ").and_then(|l| l.split_once(' ')))
        .collect();
    assert!(
        child_fds.iter().any(|(number, _)| *number == "1"),
        "the child's own stdout (fd 1) is missing from its listing -- \
         the probe observes nothing, so 'absent' below would pass vacuously: {listing}"
    );
    for (what, target) in [
        ("compositor fd", cloexec_marker.path.to_string_lossy()),
        (
            "socket pair end (or its clone)",
            pair_target.as_str().into(),
        ),
        ("socket pair end", other_target.as_str().into()),
    ] {
        assert!(
            !child_fds.iter().any(|(_, seen)| *seen == target),
            "a State::spawn child inherited a close-on-exec {what} ({target}): {listing}"
        );
    }
    // The eventfd's half of the listing: `grep -H ''` prefixes every line
    // with the fdinfo file it came from, so a count line reads
    // `/proc/<pid>/fdinfo/<n>:eventfd-count:        <hex>`.
    let fdinfo_lines: Vec<&str> = listing
        .lines()
        .filter(|line| line.starts_with("/proc/"))
        .collect();
    assert!(
        fdinfo_lines.iter().any(|line| line.contains("/fdinfo/1:")),
        "the child reported no fdinfo for its own stdout -- the eventfd check \
         below would pass vacuously: {listing}"
    );
    assert!(
        !fdinfo_lines
            .iter()
            .any(|line| eventfd_count(line) == Some(u64::from(event_count))),
        "a State::spawn child inherited a close-on-exec eventfd \
         (count {event_count:#x}): {listing}"
    );
    assert!(
        child_fds
            .iter()
            .any(|(_, seen)| *seen == plain_marker.path.to_string_lossy()),
        "the positive control ({}) is missing from the child's table -- either \
         the probe stopped observing inheritance, or Rust std started closing \
         every fd at spawn and the guarantee moved: {listing}",
        plain_marker.path.display()
    );
    // Every marker must still be open here: had one closed before the child
    // exec'd, its path or inode could have been reused and "absent"/"present"
    // would prove nothing. This is also the markers' last use, which is what
    // keeps them alive across the spawn -- without it they could drop
    // (closing the fds) before the child even starts.
    for marker in [
        cloexec_marker.fd.as_raw_fd(),
        plain_marker.fd.as_raw_fd(),
        sock_a.as_raw_fd(),
        sock_b.as_raw_fd(),
        sock_clone.as_raw_fd(),
        event.as_raw_fd(),
    ] {
        assert_ne!(
            flag_of(marker),
            -1,
            "a marker fd closed before the child's listing was read"
        );
    }
}
