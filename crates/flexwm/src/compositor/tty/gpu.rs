//! Which DRM device `--tty` drives, and whether it can actually drive a
//! display.
//!
//! Split out of `mod.rs` because picking a device turned out to be a real
//! problem rather than one `primary_gpu()` call. Smithay's
//! [`primary_gpu`] ranks candidates (1) a PCI parent with `boot_vga=1`,
//! (2) alphabetically first among those with a DRM *render* node, (3)
//! alphabetically first. On a PC that lands on the right device; on an SoC
//! whose 3D GPU and display controller are *separate* DRM devices -- Apple
//! Silicon under Asahi Linux, where `asahi`/AGX has a render node and
//! `apple,dcp` owns the CRTCs and connectors -- rule (1) can never match
//! (no PCI, no VGA BIOS) and rule (2) picks the render-only compute GPU.
//! Loading KMS resources on that device fails with `ENOTSUP`, which is
//! exactly the failure reported from real hardware (see `ROADMAP.md`).
//!
//! So: [`candidates`] returns `primary_gpu()`'s pick *first* -- it is
//! right on ordinary hardware and must keep winning there -- followed by
//! every other device on the seat, and `mod.rs`'s `init` walks that list
//! until one works. [`open`] is the per-candidate gate: it opens the
//! device through the session and proves, before anything takes ownership
//! of the file descriptor, that KMS resources load and some connector is
//! connected with a usable mode.
//!
//! Doing that check on a *borrowed* fd is the point of [`Probe`]. A
//! rejected candidate can then be handed straight back to libseat with
//! `Session::close`, which needs the `OwnedFd` -- once the fd goes into
//! Smithay's `DeviceFd` (an `Arc<OwnedFd>` with no way back out) that is
//! no longer possible. It also keeps a hopeless candidate from ever
//! constructing a `DrmDeviceFd`, whose own startup logging ("Unable to
//! become drm master...") would otherwise be emitted once per device and
//! read like the cause of the failure rather than noise.

use std::fmt::Write as _;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use smithay::backend::session::Session;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::udev::{all_gpus, primary_gpu};
use smithay::reexports::drm::Device as BasicDevice;
use smithay::reexports::drm::control::{
    Device as ControlDevice, Mode, ModeTypeFlags, ResourceHandles, connector,
};
use smithay::reexports::rustix::fs::OFlags;

/// A device that has been opened through the session and proven to have a
/// display pipeline: its KMS resources loaded and one of its connectors is
/// connected with a usable mode.
///
/// The `connector`/`mode` pair was read through [`Probe`], i.e. through a
/// different file description than the `DrmDevice` the caller goes on to
/// build. That is fine and deliberate: a `connector::Handle` is a
/// device-global kernel object id, not a per-fd token, and `Mode` is a
/// plain `Copy` description of a timing.
pub struct OpenGpu {
    /// Ownership passes to the caller, which means so does the
    /// responsibility to keep it (or to drop it, closing the fd). Every
    /// path in this module that does *not* return an `OpenGpu` has
    /// already handed the device back to the session.
    pub fd: OwnedFd,
    pub connector: connector::Handle,
    pub mode: Mode,
}

/// Every device worth trying, best guess first.
///
/// `explicit` (a `--gpu PATH`) replaces the search outright: exactly that
/// one path, no fallback, because the whole point of the flag is to be
/// obeyed on hardware whose automatic choice is wrong. Nothing here checks
/// that the path exists -- [`open`] reports what the session says about
/// it, which is both more accurate and free of a check-then-open race.
///
/// Otherwise: `primary_gpu()`'s pick first (it is the correct answer on
/// ordinary hardware, and the fallback must not demote it there), then
/// every other device on the seat in `all_gpus()`'s own sorted order.
/// `primary_gpu()` always returns one of `all_gpus()`'s entries, so
/// filtering it out of the tail is what keeps a device from being tried
/// twice.
pub fn candidates(seat: &str, explicit: Option<&Path>) -> io::Result<Vec<PathBuf>> {
    if let Some(path) = explicit {
        return Ok(vec![path.to_owned()]);
    }
    Ok(order(primary_gpu(seat)?, all_gpus(seat)?))
}

/// The ordering half of [`candidates`], split out so it can be tested
/// without a seat. See that function's doc for the rules.
fn order(primary: Option<PathBuf>, rest: Vec<PathBuf>) -> Vec<PathBuf> {
    let Some(primary) = primary else {
        return rest;
    };
    let mut ordered = Vec::with_capacity(rest.len() + 1);
    ordered.extend(rest.into_iter().filter(|path| *path != primary));
    ordered.insert(0, primary);
    ordered
}

/// Walks [`candidates`] in order and returns the first one `open` accepts,
/// or -- if none is accepted -- every candidate paired with why it was
/// rejected, in the order they were tried, ready for
/// [`unusable_device_error`].
///
/// `open` is a parameter rather than a direct call to [`open`] because
/// "usable" is not settled until `mod.rs` has built a `DrmDevice`, a
/// surface and its buffers on the device; this is the only part of that
/// sequence that can be exercised without a seat, and it is the part the
/// whole fallback rests on. Short-circuits: a candidate after the first
/// accepted one is never opened, which is what keeps ordinary single-GPU
/// hardware doing exactly one `Session::open` as it always has.
pub fn first_usable<T>(
    candidates: Vec<PathBuf>,
    mut open: impl FnMut(&Path) -> Result<T, String>,
) -> Result<(PathBuf, T), Vec<(PathBuf, String)>> {
    let mut failures = Vec::new();
    for path in candidates {
        match open(&path) {
            Ok(device) => return Ok((path, device)),
            Err(reason) => {
                // warn!, not debug!: on hardware where the first pick is
                // wrong this is the only trace of what was rejected and
                // why, and the run that follows it looks completely normal.
                tracing::warn!(path = %path.display(), reason, "drm: device unusable");
                failures.push((path, reason));
            }
        }
    }
    Err(failures)
}

/// Opens one candidate through the session and checks it can drive a
/// display. `Err` is a lowercase phrase completing "this device ...", for
/// [`unusable_device_error`] to list; a rejected device has already been
/// closed and returned to the session by the time it is returned.
pub fn open(session: &mut LibSeatSession, path: &Path) -> Result<OpenGpu, String> {
    let fd = session
        .open(path, OFlags::RDWR | OFlags::CLOEXEC)
        .map_err(|error| format!("could not be opened through the session ({error})"))?;

    match probe(fd.as_fd()) {
        Ok((connector, mode)) => Ok(OpenGpu {
            fd,
            connector,
            mode,
        }),
        Err(reason) => {
            // Back to libseat, not merely dropped: dropping closes our fd
            // but leaves seatd holding the device open for the life of the
            // process, and leaves libseat's own fd->device table with a
            // stale entry. Failing to close is not itself a reason to stop
            // trying other devices, so it is logged, not propagated.
            if let Err(error) = session.close(fd) {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "could not hand the rejected drm device back to the session"
                );
            }
            Err(reason)
        }
    }
}

/// Reads KMS state through a borrowed fd -- see this module's doc for why
/// the check happens before anything takes ownership of it.
fn probe(fd: BorrowedFd<'_>) -> Result<(connector::Handle, Mode), String> {
    let device = Probe(fd);
    // The wording stops at what the kernel actually said: the errno varies
    // with *why* there is no mode-setting pipeline (`ENOTSUP` from a driver
    // built without `DRIVER_MODESET`, which is the Apple Silicon report;
    // `EACCES` when the path is a render node, which is what the dev VM's
    // `renderD128` returns), so naming one cause would be wrong for the
    // other. The shared, provable part is that this device cannot mode-set.
    let resources = device.resource_handles().map_err(|error| {
        format!(
            "has no usable KMS pipeline -- loading its DRM resources failed \
             ({error}); a device that only computes, with no display \
             controller behind it, fails exactly here"
        )
    })?;
    find_connector_and_mode(&device, &resources)
        .ok_or_else(|| "has no connected connector with a usable mode".to_owned())
}

/// A borrowed fd viewed as a DRM device, for read-only KMS queries.
/// `drm`'s two device traits are both blanket-method-only over `AsFd`, so
/// the three impls below are the whole implementation.
struct Probe<'a>(BorrowedFd<'a>);

impl AsFd for Probe<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0
    }
}

impl BasicDevice for Probe<'_> {}
impl ControlDevice for Probe<'_> {}

/// The first `Connected` connector with at least one mode, and that mode
/// (its `PREFERRED`-flagged one if any, else its first). One output only
/// (multi-output is out of scope for this backend), so the first match
/// wins.
///
/// Generic over the device so the same search runs on a [`Probe`] here and
/// could run on a `DrmDevice`; `resources` is passed in rather than read
/// here so [`probe`] can tell "this device has no KMS at all" (the Asahi
/// failure) apart from "this device has KMS but nothing is plugged in".
fn find_connector_and_mode(
    device: &impl ControlDevice,
    resources: &ResourceHandles,
) -> Option<(connector::Handle, Mode)> {
    for &conn in resources.connectors() {
        let Ok(info) = device.get_connector(conn, false) else {
            continue;
        };
        if info.state() != connector::State::Connected {
            continue;
        }
        let mode = info
            .modes()
            .iter()
            .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| info.modes().first())
            .copied();
        if let Some(mode) = mode {
            return Some((conn, mode));
        }
    }
    None
}

/// What to tell the user when no candidate worked, given the same
/// `explicit` [`candidates`] was called with and every `(device, reason)`
/// that failed.
///
/// The old error named one device and one OS error; this keeps that detail
/// per device rather than collapsing the list into a count, so "tried two,
/// one has no KMS and the other has nothing plugged in" stays readable.
/// An explicit `--gpu` gets its own wording: there is exactly one device
/// and the user chose it, so a list of one and an offer of the flag they
/// already passed would both be noise.
pub fn unusable_device_error(
    seat: &str,
    explicit: Option<&Path>,
    failures: &[(PathBuf, String)],
) -> String {
    if let Some(path) = explicit {
        let reason = failures
            .iter()
            .find(|(failed, _)| failed == path)
            .map_or("is not usable", |(_, reason)| reason.as_str());
        // Quoted, unlike the list form below where the path starts its own
        // indented line: here it sits mid-sentence, where a path with a
        // space in it -- or the empty string, which `--gpu ""` really does
        // reach -- would otherwise dissolve into the prose around it.
        return format!("the device given by `--gpu {}` {reason}", path.display());
    }
    if failures.is_empty() {
        return format!(
            "no DRM device on seat `{seat}` -- is a GPU present and assigned \
             to this seat? (`--gpu PATH` names one explicitly)"
        );
    }
    let mut message = format!(
        "no usable DRM device on seat `{seat}`; pass `--gpu PATH` to name one \
         explicitly. Tried:"
    );
    for (path, reason) in failures {
        // Writing to a String cannot fail.
        let _ = write!(message, "\n  {} {reason}", path.display());
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn the_primary_gpu_is_tried_first_and_only_once() {
        assert_eq!(
            order(
                Some(PathBuf::from("/dev/dri/card1")),
                paths(&["/dev/dri/card0", "/dev/dri/card1", "/dev/dri/card2"]),
            ),
            paths(&["/dev/dri/card1", "/dev/dri/card0", "/dev/dri/card2"]),
        );
    }

    #[test]
    fn without_a_primary_the_seats_own_order_stands() {
        assert_eq!(
            order(None, paths(&["/dev/dri/card0", "/dev/dri/card1"])),
            paths(&["/dev/dri/card0", "/dev/dri/card1"]),
        );
    }

    #[test]
    fn one_device_stays_one_device() {
        assert_eq!(
            order(
                Some(PathBuf::from("/dev/dri/card0")),
                paths(&["/dev/dri/card0"])
            ),
            paths(&["/dev/dri/card0"]),
        );
    }

    #[test]
    fn no_devices_at_all_is_an_empty_list_not_a_phantom_entry() {
        assert!(order(None, Vec::new()).is_empty());
    }

    #[test]
    fn a_primary_missing_from_the_seat_listing_is_still_tried() {
        // Can't happen through `candidates` -- `primary_gpu` picks from
        // `all_gpus`'s own set -- but `order` must not silently drop it if
        // Smithay's two functions ever disagree.
        assert_eq!(
            order(Some(PathBuf::from("/dev/dri/card9")), Vec::new()),
            paths(&["/dev/dri/card9"]),
        );
    }

    #[test]
    fn an_explicit_gpu_replaces_the_search() {
        let chosen = Path::new("/dev/dri/card3");
        assert_eq!(
            candidates("seat0", Some(chosen)).expect("explicit paths need no seat"),
            paths(&["/dev/dri/card3"]),
        );
    }

    /// `first_usable`'s `open`, recording what it was asked to open so a
    /// test can prove a later candidate was never touched.
    fn opener<'a>(
        tried: &'a mut Vec<PathBuf>,
        works: &'a str,
    ) -> impl FnMut(&Path) -> Result<&'static str, String> + 'a {
        move |path| {
            tried.push(path.to_owned());
            if path == Path::new(works) {
                Ok("a device")
            } else {
                Err("has no usable KMS pipeline".to_owned())
            }
        }
    }

    #[test]
    fn the_first_working_device_wins_and_nothing_after_it_is_opened() {
        let mut tried = Vec::new();
        let found = first_usable(
            paths(&["/dev/dri/card0", "/dev/dri/card1"]),
            opener(&mut tried, "/dev/dri/card0"),
        );
        assert_eq!(
            found.map(|(path, _)| path),
            Ok(PathBuf::from("/dev/dri/card0"))
        );
        assert_eq!(tried, paths(&["/dev/dri/card0"]));
    }

    #[test]
    fn a_failing_first_device_falls_through_to_the_next_one() {
        // The whole point of the item: `primary_gpu`'s pick is a compute-only
        // GPU, and the display controller is the device after it.
        let mut tried = Vec::new();
        let found = first_usable(
            paths(&["/dev/dri/card0", "/dev/dri/card1", "/dev/dri/card2"]),
            opener(&mut tried, "/dev/dri/card1"),
        );
        assert_eq!(
            found.map(|(path, _)| path),
            Ok(PathBuf::from("/dev/dri/card1"))
        );
        assert_eq!(tried, paths(&["/dev/dri/card0", "/dev/dri/card1"]));
    }

    #[test]
    fn every_device_failing_reports_every_device_in_the_order_tried() {
        let mut tried = Vec::new();
        let found = first_usable(
            paths(&["/dev/dri/card1", "/dev/dri/card0"]),
            opener(&mut tried, "/dev/dri/nothing"),
        );
        let Err(failures) = found else {
            panic!("nothing should have been accepted");
        };
        assert_eq!(
            failures,
            vec![
                (
                    PathBuf::from("/dev/dri/card1"),
                    "has no usable KMS pipeline".to_owned()
                ),
                (
                    PathBuf::from("/dev/dri/card0"),
                    "has no usable KMS pipeline".to_owned()
                ),
            ]
        );
        assert_eq!(tried, paths(&["/dev/dri/card1", "/dev/dri/card0"]));
    }

    #[test]
    fn no_candidates_opens_nothing_and_reports_no_failures() {
        let mut tried = Vec::new();
        let found = first_usable(Vec::new(), opener(&mut tried, "/dev/dri/card0"));
        assert_eq!(found.map(|(path, _)| path), Err(Vec::new()));
        assert!(tried.is_empty());
    }

    #[test]
    fn the_final_error_names_every_device_and_why_each_failed() {
        let failures = vec![
            (
                PathBuf::from("/dev/dri/card0"),
                "has no KMS resources to load (os error 95)".to_owned(),
            ),
            (
                PathBuf::from("/dev/dri/card1"),
                "has no connected connector with a usable mode".to_owned(),
            ),
        ];
        let message = unusable_device_error("seat0", None, &failures);
        assert!(message.contains("seat0"), "{message}");
        assert!(message.contains("--gpu"), "{message}");
        assert!(
            message.contains("/dev/dri/card0 has no KMS resources"),
            "{message}"
        );
        assert!(
            message.contains("/dev/dri/card1 has no connected connector"),
            "{message}"
        );
    }

    #[test]
    fn one_failing_device_reads_as_a_reason_not_a_tally() {
        let failures = vec![(
            PathBuf::from("/dev/dri/card0"),
            "has no connected connector with a usable mode".to_owned(),
        )];
        let message = unusable_device_error("seat0", None, &failures);
        assert!(
            message.contains("/dev/dri/card0 has no connected connector"),
            "{message}"
        );
        // The list is the message; nothing counts how long it is, so one
        // failure reads the same way two do rather than as "tried 1 of 1".
        assert_eq!(message.matches('\n').count(), 1, "{message}");
    }

    #[test]
    fn no_devices_found_says_so_rather_than_listing_nothing() {
        let message = unusable_device_error("seat0", None, &[]);
        assert!(
            message.contains("no DRM device on seat `seat0`"),
            "{message}"
        );
        assert!(!message.contains("Tried:"), "{message}");
    }

    #[test]
    fn an_explicit_gpu_is_blamed_by_name_without_a_list() {
        let chosen = PathBuf::from("/dev/dri/card9");
        let failures = vec![(
            chosen.clone(),
            "could not be opened through the session (No such file or directory)".to_owned(),
        )];
        let message = unusable_device_error("seat0", Some(&chosen), &failures);
        assert_eq!(
            message,
            "the device given by `--gpu /dev/dri/card9` could not be opened \
             through the session (No such file or directory)"
        );
    }

    #[test]
    fn an_explicit_gpu_with_no_recorded_reason_still_reads_as_a_sentence() {
        // Defensive: `init` always records a reason for every candidate it
        // tried, so this is unreachable today -- but a `map_or` that
        // produced an empty tail would read as a truncated sentence.
        let chosen = PathBuf::from("/dev/dri/card9");
        assert_eq!(
            unusable_device_error("seat0", Some(&chosen), &[]),
            "the device given by `--gpu /dev/dri/card9` is not usable"
        );
    }

    #[test]
    fn an_empty_explicit_path_is_still_visible_in_the_message() {
        // `--gpu ""` parses, reaches the session, and is refused there. The
        // quoting is what keeps the resulting sentence from reading as if
        // no path had been named at all.
        let chosen = PathBuf::new();
        assert_eq!(
            unusable_device_error("seat0", Some(&chosen), &[]),
            "the device given by `--gpu ` is not usable"
        );
    }
}
