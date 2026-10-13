//! The Mocu cursor art scoot draws its family from.
//!
//! Every `d` string and hotspot below is copied verbatim from the vendored
//! SVG of the same name in `mocu/` (`ZOOM_IN` from `zoom-in.svg`, and so on) -- Mocu by Severin Meyer, CC0 1.0 (see
//! `mocu/README.md` and `mocu/LICENSE.txt`; credited in `NOTICE`). The
//! coordinates are Mocu's 24-unit design grid: an outline drawn at 24px is
//! one unit per pixel. A unit test checks each string against its file.
//!
//! Mocu draws every shape the same way, which is what scoot reproduces: the
//! path filled with the body paint, a 2-unit round-joined stroke of the rim
//! paint under it (so the rim reaches one unit past the fill), and a faint
//! offset shadow (not drawn here; the rim alone carries the edge).

/// One Mocu cursor: its outline paths and hotspot, in 24-grid units.
pub(crate) struct Outline {
    /// The `d` attributes of the file's filled paths, in paint order.
    pub paths: &'static [&'static str],
    /// The centre of the file's `<circle id="hot">`.
    pub hot: (f64, f64),
}

/// The classic left-pointing arrow (`default`). Also the body of the
/// `progress` and `help` art, which reuse this exact path.
pub(crate) const DEFAULT: Outline = Outline {
    paths: &["m1 1v13.75l3.94-1.63 1.72 4.16 1.84-0.78-1.71-4.15 3.94-1.63z"],
    hot: (1.0, 1.0),
};

/// The question mark of `help`, drawn beside the arrow.
pub(crate) const HELP_MARK: &str = "m14 9a4 4 0 0 0-3.9 3l1.9 0.5a2 2 0 0 1 2-1.5 2 2 0 0 1 0 0 2 2 0 0 1 2 2 2 2 0 0 1-1.5 1.9l-1.5 0.4v2.7h2v-1.2a4 4 0 0 0 3-3.8 4 4 0 0 0-4-4zm-1 10v2h2v-2h-2z";

/// The badge disk of `progress` (`<circle id="s" cx="14" cy="15"
/// r="6.125">`), in 24-grid units: centre and radius.
pub(crate) const PROGRESS_DISK: (f64, f64, f64) = (14.0, 15.0, 6.125);

/// The spinner annulus `progress` masks out of a smaller disk inside the
/// badge: inner and outer radius (its mask's `r="3"` cut-out and the masked
/// `r="4.75"` disk).
pub(crate) const PROGRESS_SPINNER: (f64, f64) = (3.0, 4.75);

pub(crate) const TEXT: Outline = Outline {
    paths: &["m9 4v1h2v13h-2v1h5v-1h-2v-13h2v-1z"],
    hot: (11.5, 11.5),
};

pub(crate) const VERTICAL_TEXT: Outline = Outline {
    paths: &["m19 9h-1v2h-13v-2h-1v5h1v-2h13v2h1z"],
    hot: (11.5, 11.5),
};

/// The pointing hand (`pointer`).
pub(crate) const POINTER: Outline = Outline {
    paths: &[
        "m7 2a1 1 0 0 0-1 1v11h-1v-4.5l-2 2c-0.5 0.5-0.5 1-0.25 1.5l2 4h9.5l1.75-3.5v-3.5a1 1 0 0 0-1-1v2h-1v-3a1 1 0 0 0-1-1h-1v3h-1v-3a1 1 0 0 0-1-1h-1v4h-1v-7a1 1 0 0 0-1-1z",
    ],
    hot: (7.0, 2.0),
};

pub(crate) const NOT_ALLOWED: Outline = Outline {
    paths: &[
        "m12 3.25a8.75 8.75 0 0 0-8.75 8.75 8.75 8.75 0 0 0 8.75 8.75 8.75 8.75 0 0 0 8.75-8.75 8.75 8.75 0 0 0-8.75-8.75zm0 2.5a6.25 6.25 0 0 1 5.21 9.7l-8.66-8.66a6.25 6.25 0 0 1 3.45-1.04zm-5.21 2.8 8.66 8.66a6.25 6.25 0 0 1-3.45 1.04 6.25 6.25 0 0 1-5.21-9.7z",
    ],
    hot: (12.0, 12.0),
};

/// The four-way move arrows (`all-scroll`).
pub(crate) const ALL_SCROLL: Outline = Outline {
    paths: &[
        "m11 1-3.5 4h7l-3.5-4zm-6 6.5-4 3.5 4 3.5v-7zm12 0v7l4-3.5-4-3.5zm-6 0.5a3 3 0 0 0-3 3 3 3 0 0 0 3 3 3 3 0 0 0 3-3 3 3 0 0 0-3-3zm-3.5 9 3.5 4 3.5-4h-7z",
    ],
    hot: (11.0, 11.0),
};

pub(crate) const ROW_RESIZE: Outline = Outline {
    paths: &["m12 1-4 5h3v3h2v-3h3l-4-5zm-8 9v2h16v-2h-16zm7 3v3h-3l4 5 4-5h-3v-3h-2z"],
    hot: (12.0, 11.0),
};

pub(crate) const COL_RESIZE: Outline = Outline {
    paths: &["m10 4v16h2v-16h-2zm-4 4-5 4 5 4v-3h3v-2h-3v-3zm10 0v3h-3v2h3v3l5-4-5-4z"],
    hot: (11.0, 12.0),
};

pub(crate) const NS_RESIZE: Outline = Outline {
    paths: &["m6 15 5.5 6 5.5-6h-4v-7h4l-5.5-6-5.5 6h4v7z"],
    hot: (11.5, 11.5),
};

pub(crate) const EW_RESIZE: Outline = Outline {
    paths: &["m8 6-6 5.5 6 5.5v-4h7v4l6-5.5-6-5.5v4h-7v-4z"],
    hot: (11.5, 11.5),
};

pub(crate) const NESW_RESIZE: Outline = Outline {
    paths: &["m18 5v8l-3-3-5 5 3 3h-8v-8l3 3 5-5-3-3z"],
    hot: (11.5, 11.5),
};

pub(crate) const NWSE_RESIZE: Outline = Outline {
    paths: &["m5 5v8l3-3 5 5-3 3h8v-8l-3 3-5-5 3-3z"],
    hot: (11.5, 11.5),
};

pub(crate) const CROSSHAIR: Outline = Outline {
    paths: &["m11 3v8h-8v1h8v8h1v-8h8v-1h-8v-8z"],
    hot: (11.5, 11.5),
};

/// The bold plus Mocu draws for `cell`.
pub(crate) const CELL: Outline = Outline {
    paths: &["m10 5v5h-5v4h5v5h4v-5h5v-4h-5v-5h-4z"],
    hot: (12.0, 12.0),
};

pub(crate) const GRAB: Outline = Outline {
    paths: &[
        "m10 1a1 1 0 0 0-1 1v7h-1v-6a1 1 0 0 0-1-1 1 1 0 0 0-1 1v10h-1l-1.7-3.4c-0.2-0.4-0.5-0.6-0.9-0.6-0.7 0-1.3 0.7-0.9 1.5l3.25 6.5h9.5l1.75-3.5v-7.5a1 1 0 0 0-1-1v5h-1v-7a1 1 0 0 0-1-1 1 1 0 0 0-1 1v6h-1v-7a1 1 0 0 0-1-1z",
    ],
    hot: (10.0, 10.0),
};

pub(crate) const GRABBING: Outline = Outline {
    paths: &[
        "m7 5a1 1 0 0 0-1 1v8h-1v-4.5l-2 2c-0.5 0.5-0.5 1-0.25 1.5l2 4h9.5l1.75-3.5v-3.5a1 1 0 0 0-1-1v2h-1v-3a1 1 0 0 0-1-1h-1v3h-1v-3a1 1 0 0 0-1-1h-1v4h-1v-4a1 1 0 0 0-1-1z",
    ],
    hot: (10.0, 10.0),
};

/// The magnifier lens and handle shared by `zoom-in` and `zoom-out`.
pub(crate) const ZOOM_LENS: &str = "m10.5 2a8.5 8.5 0 0 0-8.5 8.5 8.5 8.5 0 0 0 8.5 8.5 8.5 8.5 0 0 0 5.07-1.68l4.55 4.55c0.5 0.5 2.25-1.25 1.75-1.75l-4.55-4.55a8.5 8.5 0 0 0 1.68-5.07 8.5 8.5 0 0 0-8.5-8.5zm0 1.75a6.75 6.75 0 0 1 6.75 6.75 6.75 6.75 0 0 1-6.75 6.75 6.75 6.75 0 0 1-6.75-6.75 6.75 6.75 0 0 1 6.75-6.75z";

pub(crate) const ZOOM_IN: Outline = Outline {
    paths: &[ZOOM_LENS, "m10 7v3h-3v1h3v3h1v-3h3v-1h-3v-3z"],
    hot: (10.5, 10.5),
};

pub(crate) const ZOOM_OUT: Outline = Outline {
    paths: &[ZOOM_LENS, "m7 10v1h7v-1z"],
    hot: (10.5, 10.5),
};
