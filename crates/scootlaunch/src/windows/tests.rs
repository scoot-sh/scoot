//! Windows mode's pure parts: the rows built from announced windows and
//! the icon match. The protocol side is driven end to end against a real
//! compositor in `tests/windows.rs`.

use std::path::PathBuf;

use super::entries::{icon_for, to_entries, window_number};
use crate::index::Entry;

fn app(id: &str, icon: Option<&str>, class: Option<&str>) -> Entry {
    Entry {
        id: id.to_string(),
        name: id.to_string(),
        generic_name: None,
        comment: None,
        keywords: Vec::new(),
        icon: icon.map(str::to_string),
        exec: vec!["x".to_string()],
        terminal: false,
        dbus_activatable: false,
        actions: Vec::new(),
        path: PathBuf::from("/x"),
        workdir: None,
        startup_wm_class: class.map(str::to_string),
        hidden: None,
    }
}

#[test]
fn an_app_id_matches_its_desktop_id() {
    let apps = [app("foot.desktop", Some("foot-icon"), None)];
    assert_eq!(icon_for("foot", &apps).as_deref(), Some("foot-icon"));
}

#[test]
fn startup_wm_class_matches_when_the_id_does_not() {
    let apps = [
        app(
            "org.gnome.Terminal.desktop",
            Some("term"),
            Some("Gnome-terminal"),
        ),
        app("other.desktop", Some("other"), None),
    ];
    assert_eq!(icon_for("Gnome-terminal", &apps).as_deref(), Some("term"));
}

#[test]
fn an_exact_id_beats_a_class_and_a_case_blind_match() {
    let apps = [
        app("a.desktop", Some("by-class"), Some("firefox")),
        app("Firefox.desktop", Some("by-case"), None),
        app("firefox.desktop", Some("by-id"), None),
    ];
    assert_eq!(icon_for("firefox", &apps).as_deref(), Some("by-id"));
}

#[test]
fn a_class_beats_a_case_blind_id() {
    let apps = [
        app("Firefox.desktop", Some("by-case"), None),
        app("other.desktop", Some("by-class"), Some("firefox")),
    ];
    assert_eq!(icon_for("firefox", &apps).as_deref(), Some("by-class"));
}

#[test]
fn case_is_ignored_when_nothing_matches_exactly() {
    let apps = [app("Alacritty.desktop", Some("alac"), None)];
    assert_eq!(icon_for("alacritty", &apps).as_deref(), Some("alac"));
}

#[test]
fn a_matched_entry_without_an_icon_draws_the_tile_not_the_app_id() {
    let apps = [app("foot.desktop", None, None)];
    assert_eq!(icon_for("foot", &apps), None);
}

#[test]
fn no_match_tries_the_app_id_as_an_icon_name_when_it_is_a_valid_one() {
    assert_eq!(icon_for("mpv", &[]).as_deref(), Some("mpv"));
    assert_eq!(icon_for("", &[]), None);
    assert_eq!(icon_for("../etc/passwd", &[]), None);
    assert_eq!(icon_for("a/b", &[]), None);
}

#[test]
fn a_desktop_id_with_a_suffix_other_than_desktop_never_matches() {
    // `foot.desktop.bak` has no `.desktop` suffix to strip.
    let apps = [app("foot.desktop.bak", Some("bak"), None)];
    assert_eq!(icon_for("foot", &apps).as_deref(), Some("foot"));
}

#[test]
fn rows_carry_the_window_number_title_and_app_id() {
    let rows = to_entries([(3, "Inbox", "mail"), (7, "Notes", "")], &[]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "win:0003");
    assert_eq!(rows[0].name, "Inbox");
    assert_eq!(rows[0].generic_name.as_deref(), Some("mail"));
    assert_eq!(rows[1].id, "win:0007");
    assert_eq!(rows[1].generic_name, None);
    assert_eq!(window_number(&rows[0].id), Some(3));
    assert_eq!(window_number(&rows[1].id), Some(7));
}

#[test]
fn a_window_without_a_title_is_named_by_its_app_id_and_then_untitled() {
    let rows = to_entries([(0, "", "foot"), (1, "  ", ""), (2, "\n", "x")], &[]);
    assert_eq!(rows[0].name, "foot");
    assert_eq!(rows[1].name, "Untitled");
    assert_eq!(rows[2].name, "x");
}

#[test]
fn control_characters_in_a_title_become_spaces() {
    let rows = to_entries([(0, "a\nb\u{1b}[31mc\0d", "x")], &[]);
    assert_eq!(rows[0].name, "a b [31mc d");
}

#[test]
fn ids_sort_as_text_in_announce_order_at_the_cap() {
    // The ranker compares ids as text for equal titles.
    let rows = to_entries(
        (0..super::MAX_WINDOWS).map(|number| (number, "same", "x")),
        &[],
    );
    let mut sorted: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    sorted.sort_unstable();
    let announced: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(sorted, announced);
}

#[test]
fn no_windows_is_no_rows() {
    assert!(to_entries(std::iter::empty(), &[]).is_empty());
}

#[test]
fn only_win_ids_carry_a_window_number() {
    assert_eq!(window_number("win:0"), Some(0));
    assert_eq!(window_number("win:0012"), Some(12));
    assert_eq!(window_number("win:"), None);
    assert_eq!(window_number("win:-1"), None);
    assert_eq!(window_number("win:x"), None);
    assert_eq!(window_number("foot.desktop"), None);
    assert_eq!(window_number("win:99999999999999999999999"), None);
}

// --- the session, against a hand-made server ---------------------------

mod session {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    use wayland_client::Connection;

    use crate::windows::{Error, Windows};

    /// A server that answers every `wl_display.sync` and announces no
    /// global at all: the compositor without the protocol.
    fn serve_no_globals(mut server: UnixStream) {
        std::thread::spawn(move || {
            let mut request = [0u8; 12];
            // `get_registry` and `sync` are the only requests, 12 bytes
            // each (object, size and opcode, new id).
            while server.read_exact(&mut request).is_ok() {
                let object = u32::from_ne_bytes(request[0..4].try_into().unwrap());
                let opcode = u32::from_ne_bytes(request[4..8].try_into().unwrap()) & 0xffff;
                let new_id = u32::from_ne_bytes(request[8..12].try_into().unwrap());
                if object == 1 && opcode == 0 {
                    // The callback's `done`.
                    let mut reply = Vec::new();
                    reply.extend_from_slice(&new_id.to_ne_bytes());
                    reply.extend_from_slice(&((12u32 << 16).to_ne_bytes()));
                    reply.extend_from_slice(&0u32.to_ne_bytes());
                    if server.write_all(&reply).is_err() {
                        return;
                    }
                }
            }
        });
    }

    #[test]
    fn a_compositor_without_the_protocol_is_named_not_guessed() {
        let (client, server) = UnixStream::pair().unwrap();
        serve_no_globals(server);
        let conn = Connection::from_socket(client).unwrap();
        assert_eq!(Windows::connect(&conn).err(), Some(Error::NoManager));
    }

    #[test]
    fn a_compositor_that_never_answers_is_an_error_not_a_hang() {
        let (client, _server) = UnixStream::pair().unwrap();
        let conn = Connection::from_socket(client).unwrap();
        let started = Instant::now();
        assert_eq!(Windows::connect(&conn).err(), Some(Error::Stalled));
        // Bounded by the 2 s patience, with slack for a loaded machine.
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
