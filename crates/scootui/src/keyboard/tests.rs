//! Unit tests: layouts, hostile keymaps, repeat transitions, allocation.
//!
//! The RMLVO tests compile real system keymaps (`rules/evdev`,
//! `pc105`), so they prove the decode against the same descriptions the
//! compositor hands out -- no canned keymap text that could drift from
//! the platform. Compose uses an embedded two-sequence buffer (no
//! filesystem, no locale): hermetic and exact. The headless test in
//! `tests/keyboard.rs` covers the same layouts through a real compositor
//! and `scoot msg type`.

use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::io::AsFd;

use xkbcommon::xkb;

use super::keymap::{KeymapError, KeymapSet, MAX_KEYMAP_BYTES, read_keymap_bytes};
use super::repeat::Repeater;
use super::seat::{KeyState, KeyboardSeat};
use super::state::{Keyboard, MIN_TEXT_BYTES};

// evdev codes (`linux/input-event-codes.h`).
const KEY_1: u32 = 2;
const KEY_2: u32 = 3;
const KEY_Q: u32 = 16;
const KEY_W: u32 = 17;
const KEY_E: u32 = 18;
const KEY_Y: u32 = 21;
const KEY_A: u32 = 30;
const KEY_S: u32 = 31;
const KEY_D: u32 = 32;
const KEY_Z: u32 = 44;
const KEY_MINUS: u32 = 12;
const KEY_APOSTROPHE: u32 = 40;
const KEY_LEFTSHIFT: u32 = 42;

// Keysyms asserted exactly (Latin-1 and the one dead key used).
const SYM_A_LOWER: u32 = 0x61;
const SYM_A_UPPER: u32 = 0x41;
const SYM_DEAD_ACUTE: u32 = 0xFE51;

fn us() -> KeymapSet {
    KeymapSet::from_names("evdev", "pc105", "us", "", None).expect("the us layout compiles")
}

fn shift_mask(keymap: &KeymapSet) -> u32 {
    1 << keymap.keymap().mod_get_index("Shift")
}

fn text_of(keyboard: &mut Keyboard, evdev: u32) -> Vec<u8> {
    let mut buf = vec![0u8; MIN_TEXT_BYTES];
    let len = keyboard
        .text_into(evdev, &mut buf)
        .unwrap_or_else(|e| panic!("evdev {evdev} decodes: {e}"));
    buf.truncate(len);
    buf
}

#[test]
fn version_negotiation_bounds_the_seat() {
    use super::seat::negotiate_version;
    assert_eq!(negotiate_version(8), None);
    assert_eq!(negotiate_version(9), Some(9));
    assert_eq!(negotiate_version(10), Some(10));
    assert_eq!(negotiate_version(11), Some(10));
    assert_eq!(negotiate_version(u32::MAX), Some(10));
}

#[test]
fn us_letters_shift_levels_and_the_evdev_offset() {
    let keymap = us();
    let shift = shift_mask(&keymap);
    let mut keyboard = Keyboard::new(keymap);
    let mut buf = [0u8; MIN_TEXT_BYTES];
    // Without the +8 the wire code lands on the wrong XKB key; `a` here
    // proves the offset (XKB keycode 30 is not the A position).
    assert_eq!(keyboard.keysym(KEY_A), SYM_A_LOWER);
    assert_eq!(keyboard.text_into(KEY_A, &mut buf), Ok(1));
    assert_eq!(&buf[..1], b"a");
    keyboard.update_modifiers(shift, 0, 0, 0);
    assert_eq!(keyboard.keysym(KEY_A), SYM_A_UPPER);
    assert_eq!(keyboard.text_into(KEY_A, &mut buf), Ok(1));
    assert_eq!(&buf[..1], b"A");
    // `1` / Shift+`1`.
    keyboard.update_modifiers(0, 0, 0, 0);
    assert_eq!(text_of(&mut keyboard, KEY_1), b"1");
    keyboard.update_modifiers(shift, 0, 0, 0);
    assert_eq!(text_of(&mut keyboard, KEY_1), b"!");
    // Release-the-shift path: the mask, not the key, drives the state.
    keyboard.update_modifiers(0, 0, 0, 0);
    assert_eq!(text_of(&mut keyboard, KEY_A), b"a");
}

#[test]
fn de_swaps_y_and_z() {
    let keymap =
        KeymapSet::from_names("evdev", "pc105", "de", "", None).expect("the de layout compiles");
    let shift = shift_mask(&keymap);
    let mut keyboard = Keyboard::new(keymap);
    assert_eq!(text_of(&mut keyboard, KEY_Z), "y".as_bytes());
    assert_eq!(text_of(&mut keyboard, KEY_Y), "z".as_bytes());
    keyboard.update_modifiers(shift, 0, 0, 0);
    assert_eq!(text_of(&mut keyboard, KEY_2), b"\"");
    keyboard.update_modifiers(0, 0, 0, 0);
    assert_eq!(text_of(&mut keyboard, KEY_MINUS), "ß".as_bytes());
    keyboard.update_modifiers(shift, 0, 0, 0);
    assert_eq!(text_of(&mut keyboard, KEY_MINUS), b"?");
}

#[test]
fn fr_is_azerty() {
    let keymap =
        KeymapSet::from_names("evdev", "pc105", "fr", "", None).expect("the fr layout compiles");
    let shift = shift_mask(&keymap);
    let mut keyboard = Keyboard::new(keymap);
    assert_eq!(text_of(&mut keyboard, KEY_A), b"q");
    assert_eq!(text_of(&mut keyboard, KEY_Q), b"a");
    assert_eq!(text_of(&mut keyboard, KEY_W), b"z");
    assert_eq!(text_of(&mut keyboard, KEY_1), b"&");
    keyboard.update_modifiers(shift, 0, 0, 0);
    assert_eq!(text_of(&mut keyboard, KEY_1), b"1");
}

#[test]
fn dvorak_spot_checks() {
    let keymap = KeymapSet::from_names("evdev", "pc105", "us", "dvorak", None)
        .expect("dvorak compiles");
    let mut keyboard = Keyboard::new(keymap);
    assert_eq!(text_of(&mut keyboard, KEY_S), b"o");
    assert_eq!(text_of(&mut keyboard, KEY_D), b"e");
    assert_eq!(text_of(&mut keyboard, KEY_A), b"a");
    assert_eq!(text_of(&mut keyboard, KEY_E), b".");
}

#[test]
fn two_group_layout_decodes_both_groups() {
    let keymap = KeymapSet::from_names("evdev", "pc105", "us,ru", "", Some("grp:alt_shift_toggle"))
        .expect("us,ru compiles");
    let shift = shift_mask(&keymap);
    let mut keyboard = Keyboard::new(keymap);
    assert_eq!(text_of(&mut keyboard, KEY_A), b"a");
    // The compositor's group field is the effective layout index.
    keyboard.update_modifiers(0, 0, 0, 1);
    assert_eq!(text_of(&mut keyboard, KEY_A), "ф".as_bytes());
    assert_ne!(keyboard.keysym(KEY_A), xkb::keysyms::KEY_NoSymbol);
    keyboard.update_modifiers(shift, 0, 0, 1);
    assert_eq!(text_of(&mut keyboard, KEY_A), "Ф".as_bytes());
    keyboard.update_modifiers(0, 0, 0, 0);
    assert_eq!(text_of(&mut keyboard, KEY_A), b"a");
}

#[test]
fn caps_lock_locks_letters() {
    let keymap = us();
    let shift = shift_mask(&keymap);
    let caps = keymap.caps_index().expect("the us layout binds Lock");
    let mut keyboard = Keyboard::new(keymap);
    assert!(!keyboard.caps_lock());
    keyboard.update_modifiers(0, 0, 1 << caps, 0);
    assert!(keyboard.caps_lock());
    assert_eq!(keyboard.keysym(KEY_A), SYM_A_UPPER);
    assert_eq!(text_of(&mut keyboard, KEY_A), b"A");
    // Shift cancels Caps for letters (the keymap's ALPHABETIC type maps
    // Shift+Lock back to Level1): the state is still "caps locked" for
    // the greeter's warning, but the letter types lower.
    keyboard.update_modifiers(shift, 0, 1 << caps, 0);
    assert!(keyboard.caps_lock());
    assert_eq!(text_of(&mut keyboard, KEY_A), b"a");
    keyboard.update_modifiers(0, 0, 0, 0);
    assert!(!keyboard.caps_lock());
}

#[test]
fn num_lock_reports_the_locked_mod() {
    let keymap = us();
    let Some(num) = keymap.num_index() else {
        // Layouts bind Num Lock differently (or not at all); without the
        // modifier there is nothing to report and the query stays false.
        let keyboard = Keyboard::new(keymap);
        assert!(!keyboard.num_lock());
        return;
    };
    let mut keyboard = Keyboard::new(keymap);
    assert!(!keyboard.num_lock());
    keyboard.update_modifiers(0, 0, 1 << num, 0);
    assert!(keyboard.num_lock());
    keyboard.update_modifiers(0, 0, 0, 0);
    assert!(!keyboard.num_lock());
}

/// Two sequences, nothing else: dead+`e` and a doubled dead key. Enough
/// to prove the feed/status/reset path without depending on the host's
/// Compose file.
const COMPOSE_BUFFER: &str = "<dead_acute> <e> : \"é\" U00E9\n<dead_acute> <dead_acute> : \"´\" U00B4\n";

fn with_embedded_compose(keymap: KeymapSet) -> Keyboard {
    let table = xkb::compose::Table::new_from_buffer(
        keymap.context(),
        COMPOSE_BUFFER,
        "C",
        xkb::compose::FORMAT_TEXT_V1,
        xkb::compose::COMPILE_NO_FLAGS,
    )
    .expect("the embedded Compose buffer compiles");
    Keyboard::with_compose(keymap, table)
}

#[test]
fn compose_dead_key_then_e_is_e_acute() {
    let mut keyboard = with_embedded_compose(KeymapSet::from_names(
        "evdev", "pc105", "us", "intl", None,
    ).expect("us(intl) compiles"));
    let mut buf = [0u8; MIN_TEXT_BYTES];
    // The dead key alone types nothing but carries its keysym.
    assert_eq!(keyboard.keysym(KEY_APOSTROPHE), SYM_DEAD_ACUTE);
    assert_eq!(keyboard.text_into(KEY_APOSTROPHE, &mut buf), Ok(0));
    // Its partner completes the sequence.
    let len = keyboard
        .text_into(KEY_E, &mut buf)
        .expect("dead_acute+e composes");
    assert_eq!(&buf[..len], "é".as_bytes());
    // The state reset: a plain key after a completion types as itself.
    assert_eq!(text_of(&mut keyboard, KEY_A), b"a");
}

#[test]
fn compose_doubled_dead_key_is_the_lone_accent() {
    let mut keyboard = with_embedded_compose(KeymapSet::from_names(
        "evdev", "pc105", "us", "intl", None,
    ).expect("us(intl) compiles"));
    let mut buf = [0u8; MIN_TEXT_BYTES];
    assert_eq!(keyboard.text_into(KEY_APOSTROPHE, &mut buf), Ok(0));
    let len = keyboard
        .text_into(KEY_APOSTROPHE, &mut buf)
        .expect("dead_acute twice composes");
    assert_eq!(&buf[..len], "´".as_bytes());
}

#[test]
fn compose_cancelled_sequence_types_the_current_key() {
    // The embedded table knows only dead_acute+e and a doubled dead key:
    // dead_acute+q matches nothing, so the dead key is swallowed and `q`
    // types as itself.
    let mut intl = with_embedded_compose(
        KeymapSet::from_names("evdev", "pc105", "us", "intl", None).expect("us(intl) compiles"),
    );
    let mut buf = [0u8; MIN_TEXT_BYTES];
    assert_eq!(intl.text_into(KEY_APOSTROPHE, &mut buf), Ok(0));
    let len = intl.text_into(KEY_Q, &mut buf).expect("q types");
    assert_eq!(&buf[..len], b"q");
}

#[test]
fn compose_unavailable_is_a_loud_refusal_and_garbage_degrades() {
    let keymap = us();
    // No such locale on any host: the table refuses to build, loudly.
    assert!(
        super::state::compose_table_from_locale(&keymap, OsStr::new("xx_NO_SUCH_LOCALE_xx"))
            .is_err()
    );
    // The buffer parser, by contrast, is lenient: garbage builds an
    // empty table rather than failing. That is safe degradation, not
    // silent corruption -- no sequence can match, so every key decodes
    // to its own keysym.
    let table = xkb::compose::Table::new_from_buffer(
        keymap.context(),
        "this is not a Compose file }{{{\n",
        "C",
        xkb::compose::FORMAT_TEXT_V1,
        xkb::compose::COMPILE_NO_FLAGS,
    )
    .expect("the lenient parser builds");
    let mut keyboard = Keyboard::with_compose(keymap, table);
    assert_eq!(text_of(&mut keyboard, KEY_A), b"a");
}

#[test]
fn repeatable_keys_repeat_modifiers_do_not() {
    let keyboard = Keyboard::new(us());
    assert!(keyboard.key_repeats(KEY_A), "a letter repeats");
    assert!(
        !keyboard.key_repeats(KEY_LEFTSHIFT),
        "a modifier must not arm the timer"
    );
}

#[test]
fn hostile_keymaps_fail_loud() {
    // Empty text never reaches the library: an empty compile would decode
    // everything as nothing.
    assert!(matches!(
        KeymapSet::compile(String::new()),
        Err(KeymapError::Empty)
    ));
    assert!(matches!(
        KeymapSet::compile("not a keymap at all".to_owned()),
        Err(KeymapError::Compile)
    ));
    // Truncated at half: the library says no, loudly here.
    let full = us_text();
    let half = full.len() / 2;
    assert!(matches!(
        KeymapSet::compile(full[..half].to_owned()),
        Err(KeymapError::Compile)
    ));
    // NUL names never reach the crate's internal unwrap.
    assert!(matches!(
        KeymapSet::from_names("evdev\0", "pc105", "us", "", None),
        Err(KeymapError::Compile)
    ));
    assert!(matches!(
        KeymapSet::from_names("evdev", "pc105", "us", "", Some("a\0b")),
        Err(KeymapError::Compile)
    ));
    assert!(
        super::state::compose_table_from_locale(&us(), OsStr::new("C\0"))
            .is_err()
    );
}

fn us_text() -> String {
    let keymap = us();
    keymap.keymap().get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
}

fn memfd_with(contents: &[u8]) -> std::fs::File {
    let fd = rustix::fs::memfd_create(c"keymap-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("memfd_create works in tests");
    use std::io::Write;
    let mut file = std::fs::File::from(fd);
    file.write_all(contents).expect("the fixture writes");
    file
}

#[test]
fn keymap_fd_oversized_is_refused_before_buffering() {
    let big = vec![b'x'; MAX_KEYMAP_BYTES + 1];
    let file = memfd_with(&big);
    let error = read_keymap_bytes(file.as_fd()).expect_err("over-cap must fail");
    assert!(
        matches!(error, KeymapError::TooLarge { .. }),
        "over-cap reports TooLarge, got {error}"
    );
}

#[test]
fn keymap_fd_empty_and_invalid_utf8() {
    let empty = memfd_with(b"");
    assert!(matches!(
        read_keymap_bytes(empty.as_fd()),
        Err(KeymapError::Empty)
    ));
    let latin1 = memfd_with(b"xkb_keymap { \xff\xfe }");
    assert!(matches!(
        read_keymap_bytes(latin1.as_fd()),
        Err(KeymapError::InvalidUtf8)
    ));
}

#[test]
fn keymap_fd_short_read_compiles_to_refusal() {
    // The writer died mid-keymap (or the fd closed early): the read
    // succeeds with a prefix, and the compile refuses it loudly.
    let full = us_text();
    let file = memfd_with(&full.as_bytes()[..100]);
    let text = read_keymap_bytes(file.as_fd()).expect("a prefix still reads");
    assert!(matches!(
        KeymapSet::compile(text),
        Err(KeymapError::Compile)
    ));
}

#[test]
fn keymap_fd_unreadable_is_an_io_error() {
    // A directory fd reads as EISDIR: the Io mapping, without any unsafe
    // closed-fd juggling.
    let dir = std::fs::File::open("/tmp").expect("/tmp opens");
    assert!(matches!(
        read_keymap_bytes(dir.as_fd()),
        Err(KeymapError::Io(_))
    ));
}

#[test]
fn keymap_fd_seek_rewinds_the_shared_offset() {
    // The compositor hands every client the same open file description
    // (one shared offset). A client that read to the end must still get
    // the full keymap: the read seeks to 0 first.
    let full = us_text();
    let file = memfd_with(full.as_bytes());
    // First "client": reads the whole map from 0 (what a seek-based
    // client does), moving the shared offset to the end.
    let mut first = String::new();
    let mut first_handle = file.try_clone().expect("clone works");
    first_handle
        .seek(SeekFrom::Start(0))
        .expect("the first client rewinds");
    first_handle
        .read_to_string(&mut first)
        .expect("the first read works");
    assert_eq!(first.len(), full.len());
    // Second "client": no rewind of its own, yet gets everything.
    let second = read_keymap_bytes(file.as_fd()).expect("the rewind covers it");
    assert_eq!(second, full);
}

#[test]
fn keymap_round_trips_a_real_fd() {
    let full = us_text();
    let file = memfd_with(full.as_bytes());
    let text = read_keymap_bytes(file.as_fd()).expect("a real keymap reads");
    KeymapSet::compile(text).expect("and recompiles");
}

#[test]
fn repeater_arms_only_while_a_repeatable_key_is_held() {
    let mut repeat = Repeater::new();
    assert!(!repeat.armed());
    // No rate yet: nothing arms (the repeat_info event has not arrived).
    repeat.press(KEY_A, true);
    assert!(!repeat.armed());
    repeat.set_repeat_info(25, 200);
    repeat.press(KEY_A, true);
    assert!(repeat.armed());
    assert_eq!(repeat.held(), Some(KEY_A));
    assert!(repeat.poll_fd().is_some());
    // A non-repeatable press mid-hold leaves the repeat alone.
    repeat.press(KEY_LEFTSHIFT, false);
    assert_eq!(repeat.held(), Some(KEY_A));
    // Latest repeatable press wins.
    repeat.press(KEY_S, true);
    assert_eq!(repeat.held(), Some(KEY_S));
    // Releasing the old key changes nothing; releasing the repeat key ends it.
    repeat.release(KEY_A);
    assert!(repeat.armed());
    repeat.release(KEY_S);
    assert!(!repeat.armed());
    assert!(repeat.poll_fd().is_none());
    assert_eq!(repeat.take_expirations(), 0);
}

#[test]
fn repeater_rate_zero_never_arms() {
    let mut repeat = Repeater::new();
    repeat.set_repeat_info(0, 200);
    repeat.press(KEY_A, true);
    assert!(!repeat.armed());
}

#[test]
fn repeater_ignores_illegal_repeat_info() {
    let mut repeat = Repeater::new();
    repeat.set_repeat_info(25, 200);
    repeat.set_repeat_info(-1, 200);
    repeat.press(KEY_A, true);
    assert!(repeat.armed(), "a negative rate keeps the previous pair");
    repeat.set_repeat_info(0, -5);
    assert!(repeat.armed(), "a negative delay keeps the previous pair");
}

#[test]
fn repeater_cancels_on_focus_loss_and_keymap_change() {
    let mut repeat = Repeater::new();
    repeat.set_repeat_info(25, 200);
    repeat.press(KEY_A, true);
    assert!(repeat.armed());
    repeat.focus_lost();
    assert!(!repeat.armed());
    repeat.press(KEY_A, true);
    assert!(repeat.armed());
    repeat.keymap_changed();
    assert!(!repeat.armed());
}

#[test]
fn repeater_re_arms_on_rate_change_mid_hold() {
    let mut repeat = Repeater::new();
    repeat.set_repeat_info(25, 200);
    repeat.press(KEY_A, true);
    assert!(repeat.armed());
    repeat.set_repeat_info(30, 100);
    assert!(repeat.armed(), "a new rate applies to the live hold");
    assert_eq!(repeat.rate(), 30);
    repeat.set_repeat_info(0, 100);
    assert!(!repeat.armed(), "rate 0 ends the live hold");
}

#[test]
fn repeater_timer_expires() {
    let mut repeat = Repeater::new();
    repeat.set_repeat_info(100, 5);
    repeat.press(KEY_A, true);
    assert!(repeat.armed());
    // Generous: 5 ms delay against a 2 s budget, so loaded CI still
    // passes; the point is expiry at all, not its precision.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if repeat.take_expirations() > 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the timer never expired"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn seat_drops_keys_without_focus_or_keymap() {
    let mut seat = KeyboardSeat::new();
    let mut buf = [0u8; MIN_TEXT_BYTES];
    // No keymap and no focus: nothing decodes.
    assert_eq!(seat.key_event(KEY_A, KeyState::Pressed, &mut buf), None);
    seat.set_keymap_set(us(), false);
    assert_eq!(seat.key_event(KEY_A, KeyState::Pressed, &mut buf), None);
    seat.enter();
    let out = seat
        .key_event(KEY_A, KeyState::Pressed, &mut buf)
        .expect("a focused press decodes");
    assert_eq!(out.keysym, SYM_A_LOWER);
    assert_eq!(&buf[..out.text_len], b"a");
    assert!(!out.repeat);
    seat.leave();
    assert_eq!(seat.key_event(KEY_A, KeyState::Released, &mut buf), None);
    assert!(!seat.has_focus());
}

#[test]
fn seat_repeat_holds_and_cancels() {
    let mut seat = KeyboardSeat::new();
    seat.set_keymap_set(us(), false);
    seat.repeat_info_event(25, 200);
    seat.enter();
    let mut buf = [0u8; MIN_TEXT_BYTES];
    let out = seat
        .key_event(KEY_A, KeyState::Pressed, &mut buf)
        .expect("press decodes");
    assert!(!out.repeat);
    assert!(seat.repeat_armed());
    assert!(seat.repeat_fd().is_some());
    // Another key's release is irrelevant.
    seat.key_event(KEY_S, KeyState::Released, &mut buf);
    assert!(seat.repeat_armed());
    seat.key_event(KEY_A, KeyState::Released, &mut buf);
    assert!(!seat.repeat_armed());
    // Focus loss mid-hold ends the repeat.
    seat.key_event(KEY_A, KeyState::Pressed, &mut buf);
    assert!(seat.repeat_armed());
    seat.leave();
    assert!(!seat.repeat_armed());
}

#[test]
fn seat_keymap_change_cancels_repeat_and_bad_text_keeps_the_old() {
    let mut seat = KeyboardSeat::new();
    seat.set_keymap_set(us(), false);
    seat.repeat_info_event(25, 200);
    seat.enter();
    let mut buf = [0u8; MIN_TEXT_BYTES];
    seat.key_event(KEY_A, KeyState::Pressed, &mut buf);
    assert!(seat.repeat_armed());
    // A new keymap that refuses to compile: the old map stands (plain
    // keys still decode) but the hold is over either way.
    let garbage = memfd_with(b"not a keymap");
    assert!(seat.keymap_event(1, garbage.as_fd()).is_err());
    assert!(!seat.repeat_armed(), "a refused swap still ends the hold");
    let out = seat
        .key_event(KEY_A, KeyState::Pressed, &mut buf)
        .expect("the old map still decodes");
    assert_eq!(&buf[..out.text_len], b"a");
}

#[test]
fn seat_repeated_states_follow_the_rate() {
    let mut seat = KeyboardSeat::new();
    seat.set_keymap_set(us(), false);
    seat.enter();
    let mut buf = [0u8; MIN_TEXT_BYTES];
    // Rate nonzero: the timer owns repetition; `repeated` must not arrive
    // and is ignored if it does.
    seat.repeat_info_event(25, 200);
    assert_eq!(
        seat.key_event(KEY_A, KeyState::Repeated, &mut buf),
        None
    );
    // Rate 0: compositor-driven; the event's own key ticks as itself.
    seat.repeat_info_event(0, 200);
    let out = seat
        .key_event(KEY_A, KeyState::Repeated, &mut buf)
        .expect("rate-0 repeated states tick");
    assert!(out.repeat);
    assert_eq!(&buf[..out.text_len], b"a");
}

#[test]
fn seat_refuses_foreign_keymap_format() {
    let mut seat = KeyboardSeat::new();
    let file = memfd_with(b"");
    assert!(matches!(
        seat.keymap_event(0, file.as_fd()),
        Err(KeymapError::UnsupportedFormat(0))
    ));
}

#[test]
fn seat_truncated_buffer_is_empty_not_partial() {
    let mut keyboard = with_embedded_compose(
        KeymapSet::from_names("evdev", "pc105", "us", "intl", None).expect("us(intl) compiles"),
    );
    let mut tiny = [0u8; 1];
    // `é` is two bytes: with one byte of room the call fails rather than
    // writing a torn character.
    assert_eq!(
        keyboard.text_into(KEY_APOSTROPHE, &mut tiny),
        Ok(0),
        "the dead key alone writes nothing even in a tiny buffer"
    );
    assert_eq!(
        keyboard.text_into(KEY_E, &mut tiny),
        Err(super::state::TextError::Truncated)
    );
}

#[test]
fn wild_evdev_codes_cannot_wrap_into_valid_keys() {
    let mut keyboard = Keyboard::new(us());
    let mut buf = [0u8; MIN_TEXT_BYTES];
    assert_eq!(keyboard.keysym(u32::MAX), xkb::keysyms::KEY_NoSymbol);
    assert_eq!(
        keyboard.text_into(u32::MAX, &mut buf),
        Err(super::state::TextError::NoSymbol)
    );
    assert!(!keyboard.key_repeats(u32::MAX));
}

#[test]
fn hot_path_allocates_nothing() {
    use scootbg_mem::count_allocations;
    let keymap = us();
    let shift = shift_mask(&keymap);
    let caps = keymap.caps_index().expect("us binds Lock");
    let mut keyboard = Keyboard::new(keymap);
    let mut buf = [0u8; MIN_TEXT_BYTES];
    // Warm the state once outside the count (masks, code paths).
    keyboard.update_modifiers(shift, 0, 1 << caps, 0);
    let _ = keyboard.text_into(KEY_A, &mut buf);
    let _ = keyboard.caps_lock();
    keyboard.update_modifiers(0, 0, 0, 0);

    let (_, allocs) = count_allocations(|| {
        keyboard.update_modifiers(0, 0, 0, 0);
        let _ = keyboard.keysym(KEY_A);
        let _ = keyboard.text_into(KEY_A, &mut buf);
        let _ = keyboard.caps_lock();
        let _ = keyboard.num_lock();
        let _ = keyboard.key_repeats(KEY_A);
    });
    assert_eq!(allocs, 0, "modifiers, keysym, text and queries allocate");

    // The composed path too: feed a dead key plus its partner in one
    // closure, with the embedded table.
    let mut composed = with_embedded_compose(KeymapSet::from_names(
        "evdev", "pc105", "us", "intl", None,
    ).expect("us(intl) compiles"));
    let _ = composed.text_into(KEY_APOSTROPHE, &mut buf);
    let (_, allocs) = count_allocations(|| {
        let _ = composed.text_into(KEY_APOSTROPHE, &mut buf);
        let _ = composed.text_into(KEY_E, &mut buf);
    });
    assert_eq!(allocs, 0, "composed text allocates nothing per event");
}

#[test]
fn seat_event_path_allocates_nothing() {
    use scootbg_mem::count_allocations;
    let mut seat = KeyboardSeat::new();
    seat.set_keymap_set(us(), false);
    seat.repeat_info_event(25, 200);
    seat.enter();
    let mut buf = [0u8; MIN_TEXT_BYTES];
    let _ = seat.key_event(KEY_A, KeyState::Pressed, &mut buf);
    seat.key_event(KEY_A, KeyState::Released, &mut buf);
    let (_, allocs) = count_allocations(|| {
        let _ = seat.key_event(KEY_A, KeyState::Pressed, &mut buf);
        seat.key_event(KEY_A, KeyState::Released, &mut buf);
        seat.modifiers_event(0, 0, 0, 0);
        let _ = seat.caps_lock();
    });
    // The press arms a timerfd (a syscall, not a heap allocation) and
    // drops it on release; nothing here touches the Rust heap.
    assert_eq!(allocs, 0, "press, release and modifiers allocate");
}
