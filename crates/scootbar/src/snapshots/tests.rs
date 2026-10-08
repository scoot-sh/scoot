//! The snapshot scenes: span fills and blends on the bare canvas, and a
//! whole bar through the render path (measure, lay out, paint) in the
//! seven-segment test font at 1x and at fractional scales.

use std::fmt::Write as _;

use ab_glyph::{FontArc, FontVec};
use rustix::event::PollFlags;

use super::{Image, check};
use crate::action::{Action, Bindings, Trigger};
use crate::color::Color;
use crate::density::Scale;
use crate::layout::Section;
use crate::modules::{Class, Module, OutputView, Placed, Sources, Update, View};
use crate::outputs::{Frame, Size};
use crate::paint::{Canvas, Span};
use crate::render::{Member, Record, Scene, Style, paint};
use crate::testfont;
use crate::text::Text;
use crate::theme::Theme;

/// Every snapshot a test here compares: a file in `src/snapshots/` that
/// is not one of these fails [`no_snapshot_is_left_over`].
const NAMES: &[&str] = &[
    "fills",
    "bar-1x",
    "bar-1.5x",
    "bar-clipped-1.25x",
    "bar-rounded-1x-alpha",
    "bar-rounded-1.5x-alpha",
    "bar-translucent-1x",
    "bar-separated-1x",
    "bar-separated-groups-1x",
    "bar-hover-1x",
    "bar-hover-1.5x",
    "workspaces-pill-1x",
    "workspaces-circle-1x",
    "workspaces-circle-two-digits-1x",
    "workspaces-pill-hover-1x",
    "workspaces-colors-1x",
    "workspaces-dots-1x",
    "workspaces-disc-1x",
    "workspaces-pill-1.5x",
    "workspaces-circle-1.5x",
    "workspaces-circle-two-digits-1.5x",
    "workspaces-pill-hover-1.5x",
    "workspaces-colors-1.5x",
    "workspaces-dots-1.5x",
    "workspaces-disc-1.5x",
];

const fn gray(v: u8) -> Color {
    Color { r: v, g: v, b: v }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r, g, b }
}

/// Spans filled and clipped, and glyph coverage blended, on a 24×4
/// canvas: what every scene is made of.
#[test]
fn fills() {
    let (width, height) = (24, 4);
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let mut canvas = Canvas::new(&mut pixels, width, height).unwrap();
    let whole = Span { x: 0, width };
    canvas.fill_span(whole, rgb(0x1e, 0x1e, 0x2e));
    canvas.fill_span(Span { x: 1, width: 4 }, rgb(0xff, 0, 0));
    // Overlapping: the later fill wins where they meet.
    canvas.fill_span(Span { x: 4, width: 3 }, rgb(0, 0xff, 0));
    // Nothing: empty, past the edge, and at the far end of `u32`.
    canvas.fill_span(Span { x: 9, width: 0 }, rgb(0xff, 0xff, 0xff));
    canvas.fill_span(Span { x: 99, width: 5 }, rgb(0xff, 0xff, 0xff));
    canvas.fill_span(
        Span {
            x: u32::MAX - 1,
            width: u32::MAX,
        },
        rgb(0xff, 0xff, 0xff),
    );
    // Clipped at the right edge.
    canvas.fill_span(Span { x: 20, width: 10 }, rgb(0, 0, 0xff));
    // Coverage 0 (nothing), a quarter, a half, all of it, in a clip from
    // 9 to 17; the last two fall outside it, and outside the canvas.
    let clip = Span { x: 9, width: 8 };
    let white = gray(0xff);
    for (i, coverage) in [0u8, 64, 128, 255].into_iter().enumerate() {
        canvas.blend(10 + i as i64 * 2, 1, coverage, white, clip);
        canvas.blend(10 + i as i64 * 2, 2, coverage, rgb(0xf9, 0xe2, 0xaf), clip);
    }
    canvas.blend(8, 1, 255, white, clip);
    canvas.blend(17, 1, 255, white, clip);
    canvas.blend(-1, 0, 255, white, whole);
    canvas.blend(0, 4, 255, white, whole);
    check(
        "fills",
        "span fills (overlapping, empty, clipped) and coverage blends inside and outside a clip",
        &Image::from_xrgb(&pixels, width, height, false),
    );
}

/// A module showing fixed text, an icon and a class.
struct Label {
    text: &'static str,
    icon: Option<char>,
    class: Class,
}

impl Module for Label {
    fn sources<'fd>(&'fd self, _: &mut Sources<'_, 'fd>) {}

    fn on_ready(&mut self, _: usize, _: PollFlags) -> Update {
        Update::Unchanged
    }

    fn view(&self, _: &OutputView<'_>, view: &mut View) {
        let _ = view.text_mut().write_str(self.text);
        view.set_icon(self.icon);
        view.set_class(self.class);
    }
}

/// Three sections: the time on the left, an unknown character (`.notdef`,
/// a block) muted in the center, and an icon with text on the right, in
/// gray tokens so the image is one sample a pixel.
fn bar(width: u32, height: u32, scale: Scale) -> Image {
    let (pixels, w, h) = draw(width, height, scale, Look::default());
    Image::from_xrgb(&pixels, w, h, true)
}

/// What a scene's bar looks like beyond the layout: the options
/// `[bar] radius` and `opacity` set.
#[derive(Clone, Copy)]
struct Look {
    background: Color,
    radius: u32,
    opacity: u8,
    /// The separator's width; with a nonzero one the scene has all three
    /// modules on the left, so there are gaps to draw it in.
    separator: u32,
    /// Only the gap between the second and third module holds a line (as a
    /// `["a", "b", "|", "c"]` list does): the first two are one group.
    grouped: bool,
    /// The middle module's margin.
    margin: u32,
    /// The pointer over the right module: its text in the `hover` token,
    /// set apart from the `accent` one, instead of its class's.
    hover: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            background: gray(0),
            radius: 0,
            opacity: u8::MAX,
            separator: 0,
            grouped: false,
            margin: 0,
            hover: false,
        }
    }
}

/// The bar drawn through the render path: its pixels and device size.
fn draw(width: u32, height: u32, scale: Scale, look: Look) -> (Vec<u8>, u32, u32) {
    let modules = [
        (Section::Left, "12:34", None, Class::Normal),
        (Section::Center, "a Z", None, Class::Muted),
        (Section::Right, "09 pm", Some('8'), Class::Warn),
    ];
    // A hovered bar binds the hovered module, as a bound module is the
    // only one the pointer tints.
    let mut bound = Bindings::default();
    if look.hover {
        bound.set(Trigger::Click, Action::Exec(vec!["true".into()]));
    }
    let placed: Vec<Placed> = modules
        .into_iter()
        .enumerate()
        .map(|(module, (_, text, icon, class))| Placed {
            bindings: if look.hover && module == 2 {
                bound.clone()
            } else {
                Bindings::default()
            },
            id: "label",
            module: Box::new(Label { text, icon, class }),
            revision: 0,
        })
        .collect();
    let style = Style {
        theme: Theme {
            background: look.background,
            foreground: gray(0xff),
            accent: gray(0xc0),
            hover: if look.hover { gray(0xe0) } else { gray(0xc0) },
            dim: gray(0x80),
            urgent: gray(0xe0),
        },
        font_size: 14,
        padding: 4,
        spacing: if look.separator > 0 { 10 } else { 6 },
        separator: look.separator,
        radius: look.radius,
        popup_radius: 0,
        opacity: look.opacity,
    };
    let frame = Frame {
        size: Size { width, height },
        scale,
    };
    let (device_width, device_height) = scale.buffer(frame.size).unwrap();
    let mut text = Text::new(FontArc::new(
        FontVec::try_from_vec(testfont::build()).unwrap(),
    ));
    let together = look.separator > 0;
    let sections: Vec<Section> = modules
        .iter()
        .map(|m| if together { Section::Left } else { m.0 })
        .collect();
    let members: Vec<Member> = modules
        .iter()
        .enumerate()
        .map(|(module, _)| Member {
            hover: look.hover && module == 2,
            module,
            section: sections[module],
            margin: if module == 1 { look.margin } else { 0 },
            separator_before: if look.grouped {
                module == 2
            } else {
                module > 0 && sections[module - 1] == sections[module]
            },
        })
        .collect();
    let mut scene = Scene::with_members(&members);
    scene.update(
        &placed,
        &OutputView { name: None },
        Some(&text),
        &style,
        scale,
        Size {
            width: device_width,
            height: device_height,
        },
    );
    if look.hover {
        // Over the right module's middle: it alone draws tinted.
        let spans = scene.spans();
        let span = spans[2];
        scene.set_pointer(Some(span.x + span.width / 2));
    }
    let mut pixels = vec![0xaa; device_width as usize * device_height as usize * 4];
    let mut canvas = Canvas::new(&mut pixels, device_width, device_height).unwrap();
    let mut record = Record::new(placed.len());
    paint(
        &mut canvas,
        &mut record,
        &scene,
        &placed,
        Some(&mut text),
        &style,
        frame,
        &OutputView { name: None },
    );
    (pixels, device_width, device_height)
}

#[test]
fn bar_at_1x() {
    check(
        "bar-1x",
        "a 160x20 bar at scale 1: three sections, an icon, .notdef, three classes",
        &bar(160, 20, Scale::Integer(1)),
    );
}

/// 1.5: the em (21 px), the padding and the spacing all land on whole
/// pixels, but the glyphs' edges do not.
#[test]
fn bar_at_1_5x() {
    check(
        "bar-1.5x",
        "the same bar at scale 1.5 (fractional, 240x30 device pixels)",
        &bar(160, 20, Scale::Fractional(180)),
    );
}

/// Narrower than the modules: the sections overlap and are clipped (the
/// layout's rule), at a scale where nothing lands on a whole pixel.
#[test]
fn bar_clipped_at_1_25x() {
    check(
        "bar-clipped-1.25x",
        "the bar at 70x20 and scale 1.25, too narrow: its sections clipped",
        &bar(70, 20, Scale::Fractional(150)),
    );
}

/// Rounded corners at a whole scale: the alpha plane, so what is cut away
/// shows (the color plane cannot say). The radius 6 is nearly half this
/// 20-pixel bar's height.
#[test]
fn bar_rounded_at_1x() {
    let look = Look {
        radius: 6,
        ..Look::default()
    };
    let (pixels, w, h) = draw(160, 20, Scale::Integer(1), look);
    check(
        "bar-rounded-1x-alpha",
        "the alpha plane of the 160x20 bar at scale 1 with radius 6: corners cut, edge antialiased",
        &Image::from_alpha(&pixels, w, h),
    );
}

/// The same radius at 1.5, where the device radius is 9 and the edge
/// lands between pixels; half opacity, so the plane also shows the
/// background's alpha (128) inside the corners.
#[test]
fn bar_rounded_translucent_at_1_5x() {
    let look = Look {
        radius: 6,
        opacity: 128,
        ..Look::default()
    };
    let (pixels, w, h) = draw(160, 20, Scale::Fractional(180), look);
    check(
        "bar-rounded-1.5x-alpha",
        "the alpha plane at scale 1.5, radius 6 (9 device pixels), opacity 128",
        &Image::from_alpha(&pixels, w, h),
    );
}

/// Separators and a margin: the three modules together on the left with
/// 10 pixels of spacing, a 2-pixel line in each gap (the `dim` token, the
/// middle half of the bar's height), and the middle module's 3-pixel
/// margin widening both gaps beside it.
#[test]
fn bar_with_separators_and_a_margin_at_1x() {
    let look = Look {
        separator: 2,
        margin: 3,
        ..Look::default()
    };
    let (pixels, w, h) = draw(160, 20, Scale::Integer(1), look);
    check(
        "bar-separated-1x",
        "the 160x20 bar at scale 1, all modules left, spacing 10, separator 2, middle margin 3",
        &Image::from_xrgb(&pixels, w, h, true),
    );
}

/// Grouped separators: the first two modules are one group (no line), the
/// mark before the third draws the only line, as a
/// `["a", "b", "|", "c"]` list does.
#[test]
fn bar_with_grouped_separators_at_1x() {
    let look = Look {
        separator: 2,
        grouped: true,
        ..Look::default()
    };
    let (pixels, w, h) = draw(160, 20, Scale::Integer(1), look);
    check(
        "bar-separated-groups-1x",
        "the 160x20 bar at scale 1, all modules left, spacing 10, separator 2, one line between the second and third module",
        &Image::from_xrgb(&pixels, w, h, true),
    );
}

/// A translucent background with text blended over it, premultiplied:
/// the color plane is the background at half alpha, and the glyph
/// coverage mixes full white into it.
#[test]
fn bar_translucent_at_1x() {
    let look = Look {
        background: gray(0x80),
        opacity: 128,
        ..Look::default()
    };
    let (pixels, w, h) = draw(160, 20, Scale::Integer(1), look);
    check(
        "bar-translucent-1x",
        "the color plane of the bar at scale 1 over a 0x80 background at opacity 128, premultiplied",
        &Image::from_xrgb(&pixels, w, h, true),
    );
}

/// The pointer over the bound right module: its `Warn` text in the
/// `hover` token set apart from the `accent` one, the other modules in
/// their classes'.
#[test]
fn bar_hovered_at_1x() {
    let look = Look {
        hover: true,
        ..Look::default()
    };
    let (pixels, w, h) = draw(160, 20, Scale::Integer(1), look);
    check(
        "bar-hover-1x",
        "the 160x20 bar at scale 1 with the pointer over its bound right module",
        &Image::from_xrgb(&pixels, w, h, true),
    );
}

/// The hovered bar at 1.5, where the glyphs' edges land between pixels.
#[test]
fn bar_hovered_at_1_5x() {
    let look = Look {
        hover: true,
        ..Look::default()
    };
    let (pixels, w, h) = draw(160, 20, Scale::Fractional(180), look);
    check(
        "bar-hover-1.5x",
        "the hovered bar of bar-hover-1x at scale 1.5 (fractional, 240x30 device pixels)",
        &Image::from_xrgb(&pixels, w, h, true),
    );
}

/// Snapshots round-trip through their text form, and malformed ones are
/// refused rather than read as something else.
#[test]
fn the_image_format_round_trips_and_refuses_the_malformed() {
    let image = Image {
        width: 3,
        height: 2,
        gray: false,
        samples: (0..18).collect(),
    };
    assert_eq!(Image::from_pnm(&image.to_pnm("a\nb")), Ok(image));
    let gray = Image::from_xrgb(&[7, 7, 7, 0, 9, 9, 9, 0], 2, 1, true);
    assert_eq!(gray.samples, [7, 9]);
    assert_eq!(Image::from_pnm(&gray.to_pnm("")), Ok(gray));
    for bad in [
        "",
        "P5\n1 1\n255\n0",
        "P2\n1 1\n65535\n0",
        "P2\n2 1\n255\n0",
        "P2\n1 1\n255\n0 0",
        "P2\n1 1\n255\n256",
        "P2\n99999 99999\n255\n",
        "P2\nx 1\n255\n0",
    ] {
        assert!(Image::from_pnm(bad).is_err(), "{bad:?}");
    }
}

#[test]
#[should_panic(expected = "is not gray")]
fn a_colored_pixel_in_a_gray_scene_is_refused() {
    let _ = Image::from_xrgb(&[1, 2, 3, 0], 1, 1, true);
}

/// Every file in `src/snapshots/` belongs to a test above, so a renamed
/// or removed scene does not leave an image nothing compares.
#[test]
fn no_snapshot_is_left_over() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/snapshots");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        if name == "tests.rs" || name == "icons.rs" {
            continue;
        }
        let stem = name
            .strip_suffix(".pgm")
            .or_else(|| name.strip_suffix(".ppm"))
            .unwrap_or_else(|| panic!("{name}: not a snapshot"));
        assert!(
            NAMES.contains(&stem) || super::icons::NAMES.contains(&stem),
            "{name}: no test compares it"
        );
    }
}
