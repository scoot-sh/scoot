//! Shared harness for the keyboard-stack spike (`scootui-keyboard-spike`).
//!
//! Both backend bins (`spike-kbvm`, `spike-xkb`) print the same line format
//! so their outputs diff directly into the ticket's pass/fail table:
//!
//! ```text
//! layout=<id> evdev=<n> shift=<0|1> group=<n> keysym=<name> text=<escaped>
//! ```
//!
//! `text` is the decoded string with `\`, newline and non-printables
//! backslash-escaped, so an empty decode is visible as `text=` and a
//! dead key (no text) is distinguishable from a failed lookup.

/// One scripted key press: an evdev code plus the effective modifiers and
/// group a Wayland client would have from the compositor's `modifiers` event.
pub struct Probe {
    pub evdev: u32,
    pub shift: bool,
    pub group: u32,
}

/// One layout under test: the RMLVO names plus the probes to run through it.
pub struct LayoutCase {
    /// Short id used in output lines.
    pub id: &'static str,
    pub layout: &'static str,
    pub variant: &'static str,
    pub options: Option<&'static str>,
    pub probes: &'static [Probe],
}

const P: Probe = Probe {
    evdev: 0,
    shift: false,
    group: 0,
};

/// Evdev codes (linux/input-event-codes.h) used below.
pub mod ev {
    pub const N1: u32 = 2;
    pub const N2: u32 = 3;
    pub const MINUS: u32 = 12;
    pub const EQUAL: u32 = 13;
    pub const Q: u32 = 16;
    pub const W: u32 = 17;
    pub const E: u32 = 18;
    pub const A: u32 = 30;
    pub const S: u32 = 31;
    pub const D: u32 = 32;
    pub const Y: u32 = 21;
    pub const Z: u32 = 44;
    pub const APOS: u32 = 40;
}

fn probe(evdev: u32, shift: bool, group: u32) -> Probe {
    Probe { evdev, shift, group }
}

/// The ticket's layout set: `us`, `us(intl)` dead keys, `de`, `fr` (AZERTY),
/// `dvorak`, and two-group `us,ru` with `grp:alt_shift_toggle`.
pub static LAYOUTS: &[LayoutCase] = &[
    LayoutCase {
        id: "us",
        layout: "us",
        variant: "",
        options: None,
        probes: &[
            Probe { evdev: ev::A, ..P },
            Probe { evdev: ev::A, shift: true, ..P },
            Probe { evdev: ev::N1, ..P },
            Probe { evdev: ev::N1, shift: true, ..P },
            Probe { evdev: ev::APOS, ..P },
            Probe { evdev: ev::EQUAL, shift: true, ..P },
        ],
    },
    LayoutCase {
        id: "us-intl",
        layout: "us",
        variant: "intl",
        options: None,
        probes: &[
            // The apostrophe key is a dead acute on us(intl): no text alone.
            Probe { evdev: ev::APOS, ..P },
            Probe { evdev: ev::Z, ..P },
            Probe { evdev: ev::A, shift: true, ..P },
        ],
    },
    LayoutCase {
        id: "de",
        layout: "de",
        variant: "",
        options: None,
        probes: &[
            // Z and Y are swapped vs us.
            Probe { evdev: ev::Z, ..P },
            Probe { evdev: ev::Y, ..P },
            Probe { evdev: ev::N2, shift: true, ..P },
            Probe { evdev: ev::MINUS, ..P },
            Probe { evdev: ev::A, ..P },
        ],
    },
    LayoutCase {
        id: "fr",
        layout: "fr",
        variant: "",
        options: None,
        probes: &[
            // AZERTY: A and Q swap, W and Z swap, digits need Shift.
            Probe { evdev: ev::A, ..P },
            Probe { evdev: ev::Q, ..P },
            Probe { evdev: ev::W, ..P },
            Probe { evdev: ev::N1, ..P },
            Probe { evdev: ev::N1, shift: true, ..P },
        ],
    },
    LayoutCase {
        id: "dvorak",
        layout: "us",
        variant: "dvorak",
        options: None,
        probes: &[
            // QWERTY S position holds O, D holds E, A holds A.
            Probe { evdev: ev::S, ..P },
            Probe { evdev: ev::D, ..P },
            Probe { evdev: ev::A, ..P },
            Probe { evdev: ev::E, ..P },
        ],
    },
    LayoutCase {
        id: "us-ru",
        layout: "us,ru",
        variant: "",
        options: Some("grp:alt_shift_toggle"),
        probes: &[
            Probe { evdev: ev::A, group: 0, ..P },
            // After one group toggle the same key is Cyrillic ef.
            Probe { evdev: ev::A, group: 1, ..P },
            Probe { evdev: ev::A, shift: true, group: 1, ..P },
            Probe { evdev: ev::N1, group: 1, ..P },
        ],
    },
];

/// Escape a decoded string for single-line output.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Mean of a sample in milliseconds.
pub fn mean_ms(samples: &[f64]) -> f64 {
    samples.iter().sum::<f64>() / samples.len() as f64
}

pub mod wayland;

/// Silence the unused-`P` warning if a layout drops struct-update syntax.
#[allow(dead_code)]
pub fn _use_p() -> Probe {
    probe(0, false, 0)
}
