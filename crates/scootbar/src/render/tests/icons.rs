//! Path and image icons through the render path: what they add to a
//! module's width, where they are drawn and in which token, and that a
//! warm repaint of one allocates nothing.

use std::fmt::Write as _;
use std::sync::Arc;

use ab_glyph::{FontArc, FontVec};
use rustix::event::PollFlags;

use crate::color::Color;
use crate::density::Scale;
use crate::icon::path::{Vector, ViewBox};
use crate::icon::{Art, Icon};
use crate::layout::Section;
use crate::modules::{Class, Module, OutputView, Placed, Sources, Update, View};
use crate::outputs::{Frame, Size};
use crate::paint::Canvas;
use crate::render::{Record, Scene, Style, paint};
use crate::testfont;
use crate::text::Text;
use crate::theme::Theme;

const WIDTH: u32 = 400;
const HEIGHT: u32 = 60;
const EM: u32 = 50;
const PADDING: u32 = 10;

struct Iconic {
    text: &'static str,
    icon: Icon,
    class: Class,
}

impl Module for Iconic {
    fn sources<'fd>(&'fd self, _: &mut Sources<'_, 'fd>) {}

    fn on_ready(&mut self, _: usize, _: PollFlags) -> Update {
        Update::Unchanged
    }

    fn view(&self, _: &OutputView<'_>, view: &mut View) {
        let _ = view.text_mut().write_str(self.text);
        view.show_icon(&self.icon);
        view.set_class(self.class);
    }
}

fn square() -> Icon {
    Icon::Art(Art::Vector(Arc::new(
        Vector::parse("M0 0H24V24H0z", ViewBox::default()).unwrap(),
    )))
}

fn style() -> Style {
    Style {
        theme: Theme::default(),
        font_size: EM,
        padding: PADDING,
        spacing: 0,
        separator: 0,
        radius: 0,
        opacity: u8::MAX,
    }
}

fn font() -> Text {
    Text::new(FontArc::new(
        FontVec::try_from_vec(testfont::build()).unwrap(),
    ))
}

fn frame() -> Frame {
    Frame {
        size: Size {
            width: WIDTH,
            height: HEIGHT,
        },
        scale: Scale::Integer(1),
    }
}

/// One module, measured: its span's width.
fn width_of(shown: &'static str, icon: Icon) -> u32 {
    let placed = [Placed {
        id: "iconic",
        module: Box::new(Iconic {
            text: shown,
            icon,
            class: Class::Normal,
        }),
        revision: 0,
    }];
    let mut scene = Scene::all(&[Section::Left]);
    scene.update(
        &placed,
        &OutputView { name: None },
        Some(&font()),
        &style(),
        Scale::Integer(1),
        Size {
            width: WIDTH,
            height: HEIGHT,
        },
    );
    scene.spans()[0].width
}

#[test]
fn an_icon_adds_its_square_and_a_space_before_the_text() {
    let plain = font();
    let em = EM as f32;
    let digits = plain.measure(None, "12", em);
    let gap = plain.art_gap(em);
    assert!(gap > 0);
    let side = Text::art_side(em);
    assert_eq!(side, EM);
    // padding is both sides
    assert_eq!(width_of("12", square()), side + gap + digits + 2 * PADDING);
    // With no text there is no gap.
    assert_eq!(width_of("", square()), side + 2 * PADDING);
    // A glyph icon measures as before: the character and a space, in the
    // font (the test font's '8' is a digit cell).
    let glyph = width_of("12", Icon::Glyph('8'));
    assert_eq!(glyph, plain.measure(Some('8'), "12", em) + 2 * PADDING);
}

/// Paints one module and returns the canvas pixels.
fn painted(icon: Icon, text_: &'static str, class: Class) -> Vec<u8> {
    let placed = [Placed {
        id: "iconic",
        module: Box::new(Iconic {
            text: text_,
            icon,
            class,
        }),
        revision: 0,
    }];
    let style = style();
    let mut font = font();
    let mut scene = Scene::all(&[Section::Left]);
    let output = OutputView { name: None };
    scene.update(
        &placed,
        &output,
        Some(&font),
        &style,
        Scale::Integer(1),
        Size {
            width: WIDTH,
            height: HEIGHT,
        },
    );
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, WIDTH, HEIGHT).unwrap();
    let mut record = Record::new(1);
    paint(
        &mut canvas,
        &mut record,
        &scene,
        &placed,
        Some(&mut font),
        &style,
        frame(),
        &output,
    );
    pixels
}

fn at(pixels: &[u8], x: u32, y: u32) -> Color {
    let i = ((y * WIDTH + x) * 4) as usize;
    Color {
        r: pixels[i + 2],
        g: pixels[i + 1],
        b: pixels[i],
    }
}

#[test]
fn a_path_icon_is_drawn_at_the_padding_centered_in_the_class_token() {
    let theme = Theme::default();
    for (class, token) in [
        (Class::Normal, theme.foreground),
        (Class::Warn, theme.accent),
        (Class::Urgent, theme.urgent),
        (Class::Muted, theme.dim),
    ] {
        let pixels = painted(square(), "", class);
        // The 50-pixel square starts at the padding, and is 5 rows below
        // the top of the 60-pixel bar.
        for (x, y, inside) in [
            (PADDING, 5, true),
            (PADDING + 49, 54, true),
            (PADDING + 25, 30, true),
            (PADDING - 1, 30, false),
            (PADDING + 50, 30, false),
            (PADDING + 25, 4, false),
            (PADDING + 25, 55, false),
        ] {
            let want = if inside { token } else { theme.background };
            assert_eq!(at(&pixels, x, y), want, "{class:?} at ({x}, {y})");
        }
    }
}

#[test]
fn the_text_starts_after_the_icon_and_its_gap() {
    let pixels = painted(square(), "8", Class::Normal);
    let theme = Theme::default();
    let plain = font();
    let start = PADDING + EM + plain.art_gap(EM as f32);
    // The digit's ink is to the right of `start`, and nothing but the
    // icon's is left of it, past the icon.
    let ink = |x: u32| (0..HEIGHT).any(|y| at(&pixels, x, y) != theme.background);
    assert!(!ink(PADDING + EM + 1), "gap is blank");
    assert!(!ink(start - 1));
    assert!(
        (start..start + 40).any(ink),
        "the digit is drawn after the gap"
    );
}

#[test]
fn an_icon_is_clipped_to_a_span_narrower_than_it() {
    // A bar narrower than the icon: nothing past the canvas, no panic.
    let placed = [Placed {
        id: "iconic",
        module: Box::new(Iconic {
            text: "",
            icon: square(),
            class: Class::Normal,
        }),
        revision: 0,
    }];
    let style = style();
    let mut font = font();
    let mut scene = Scene::all(&[Section::Left]);
    let output = OutputView { name: None };
    let size = Size {
        width: 30,
        height: HEIGHT,
    };
    scene.update(
        &placed,
        &output,
        Some(&font),
        &style,
        Scale::Integer(1),
        size,
    );
    let mut pixels = vec![0u8; (30 * HEIGHT * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, 30, HEIGHT).unwrap();
    let mut record = Record::new(1);
    paint(
        &mut canvas,
        &mut record,
        &scene,
        &placed,
        Some(&mut font),
        &style,
        Frame {
            size,
            scale: Scale::Integer(1),
        },
        &output,
    );
}

#[test]
fn a_warm_repaint_of_an_icon_allocates_nothing() {
    let placed = [Placed {
        id: "iconic",
        module: Box::new(Iconic {
            text: "12",
            icon: square(),
            class: Class::Normal,
        }),
        revision: 0,
    }];
    let style = style();
    let mut font = font();
    let mut scene = Scene::all(&[Section::Left]);
    let output = OutputView { name: None };
    scene.update(
        &placed,
        &output,
        Some(&font),
        &style,
        Scale::Integer(1),
        Size {
            width: WIDTH,
            height: HEIGHT,
        },
    );
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, WIDTH, HEIGHT).unwrap();
    let mut record = Record::new(1);
    let mut draw = |canvas: &mut Canvas<'_>, record: &mut Record| {
        paint(
            canvas,
            record,
            &scene,
            &placed,
            Some(&mut font),
            &style,
            frame(),
            &output,
        );
    };
    // The first paint fills the icon cache and the glyph cache.
    draw(&mut canvas, &mut record);
    record.reset();
    let ((), allocations) = scootbg_mem::count_allocations(|| draw(&mut canvas, &mut record));
    assert_eq!(allocations, 0, "a warm repaint allocated");
}
