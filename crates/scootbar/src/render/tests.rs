//! Pure-drawing tests of the whole path from views to pixels, with the
//! seven-segment test font: what is measured, laid out, repainted and
//! damaged, read back from the pixels.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::rc::Rc;

use ab_glyph::{FontArc, FontVec};
use rustix::event::PollFlags;

use super::{Record, Scene, Style, damage, paint};
use crate::density::Scale;
use crate::layout::Section;
use crate::modules::{Class, Module, OutputView, Placed, Sources, Update, View};
use crate::outputs::{Frame, Size};
use crate::paint::{Canvas, Span};
use crate::testfont;
use crate::text::Text;
use crate::theme::Theme;

mod icons;
mod spacing;

/// A module whose view the test sets, counting how often it is asked.
struct Fixed {
    shown: Rc<RefCell<(String, Class)>>,
    asked: Rc<Cell<u32>>,
}

impl Module for Fixed {
    fn sources<'fd>(&'fd self, _: &mut Sources<'_, 'fd>) {}

    fn on_ready(&mut self, _: usize, _: PollFlags) -> Update {
        Update::Changed
    }

    fn view(&self, _: &OutputView<'_>, view: &mut View) {
        self.asked.set(self.asked.get() + 1);
        let shown = self.shown.borrow();
        let _ = view.text_mut().write_str(&shown.0);
        view.set_class(shown.1);
    }
}

struct Bar {
    placed: Vec<Placed>,
    shown: Vec<Rc<RefCell<(String, Class)>>>,
    asked: Vec<Rc<Cell<u32>>>,
    scene: Scene,
    text: Text,
    style: Style,
}

const WIDTH: u32 = 600;
const HEIGHT: u32 = 60;
const EM: u32 = 50;
const BASELINE: i64 = 45;

fn frame() -> Frame {
    Frame {
        size: Size {
            width: WIDTH,
            height: HEIGHT,
        },
        scale: Scale::Integer(1),
    }
}

impl Bar {
    fn new(modules: &[(Section, &str)]) -> Self {
        let mut placed = Vec::new();
        let mut shown = Vec::new();
        let mut asked = Vec::new();
        for &(_, text) in modules {
            let cell = Rc::new(RefCell::new((text.to_owned(), Class::Normal)));
            let count = Rc::new(Cell::new(0));
            placed.push(Placed {
                id: "fixed",
                module: Box::new(Fixed {
                    shown: cell.clone(),
                    asked: count.clone(),
                }),
                revision: 0,
            });
            shown.push(cell);
            asked.push(count);
        }
        let scene = Scene::all(&modules.iter().map(|m| m.0).collect::<Vec<_>>());
        let font = FontArc::new(FontVec::try_from_vec(testfont::build()).unwrap());
        Self {
            placed,
            shown,
            asked,
            scene,
            text: Text::new(font),
            style: Style {
                theme: Theme::default(),
                font_size: EM,
                padding: 10,
                spacing: 0,
                separator: 0,
                radius: 0,
                opacity: u8::MAX,
            },
        }
    }

    fn set(&mut self, index: usize, text: &str, class: Class) {
        *self.shown[index].borrow_mut() = (text.to_owned(), class);
        self.placed[index].revision += 1;
    }

    fn update(&mut self) {
        self.scene.update(
            &self.placed,
            &OutputView { name: None },
            Some(&self.text),
            &self.style,
            Scale::Integer(1),
            Size {
                width: WIDTH,
                height: HEIGHT,
            },
        );
    }

    fn paint(&mut self, pixels: &mut [u8], record: &mut Record) {
        self.update();
        let mut canvas = Canvas::new(pixels, WIDTH, HEIGHT).unwrap();
        paint(
            &mut canvas,
            record,
            &self.scene,
            &self.placed,
            Some(&mut self.text),
            &self.style,
            frame(),
            &OutputView { name: None },
        );
    }
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * WIDTH + x) * 4) as usize;
    [pixels[i + 2], pixels[i + 1], pixels[i]]
}

fn rgb(color: crate::color::Color) -> [u8; 3] {
    [color.r, color.g, color.b]
}

/// The text in `span`, read back.
fn read(pixels: &[u8], span: Span, background: [u8; 3]) -> String {
    let ink = |x: i64, y: i64| {
        x >= i64::from(span.x)
            && x < i64::from(span.end())
            && (0..i64::from(HEIGHT)).contains(&y)
            && pixel(pixels, x as u32, y as u32) != background
    };
    let columns: Vec<i64> = (i64::from(span.x)..i64::from(span.end()))
        .filter(|&x| (0..i64::from(HEIGHT)).any(|y| ink(x, y)))
        .collect();
    let (Some(&left), Some(&right)) = (columns.first(), columns.last()) else {
        return String::new();
    };
    testfont::decode(ink, left, right, BASELINE, f64::from(EM))
}

fn buffer() -> Vec<u8> {
    vec![0u8; (WIDTH * HEIGHT * 4) as usize]
}

#[test]
fn a_scene_measures_and_lays_out_its_modules() {
    let mut bar = Bar::new(&[
        (Section::Left, "1"),
        (Section::Center, "3:07"),
        (Section::Right, ""),
    ]);
    bar.update();
    // 30 a glyph plus 10 of padding a side; the empty one takes nothing.
    assert_eq!(
        bar.scene.spans(),
        [
            Span { x: 0, width: 50 },
            Span { x: 230, width: 140 },
            Span { x: 600, width: 0 },
        ]
    );
    assert_eq!(bar.scene.views()[1].text(), "3:07");
}

#[test]
fn a_view_is_asked_only_after_a_change() {
    let mut bar = Bar::new(&[(Section::Left, "1"), (Section::Right, "2")]);
    bar.update();
    bar.update();
    assert_eq!((bar.asked[0].get(), bar.asked[1].get()), (1, 1));
    bar.set(1, "22", Class::Normal);
    bar.update();
    assert_eq!((bar.asked[0].get(), bar.asked[1].get()), (1, 2));
    // A new scale or width measures everything again.
    bar.scene.update(
        &bar.placed,
        &OutputView { name: None },
        Some(&bar.text),
        &bar.style,
        Scale::Fractional(180),
        Size {
            width: WIDTH,
            height: HEIGHT,
        },
    );
    assert_eq!((bar.asked[0].get(), bar.asked[1].get()), (2, 3));
}

#[test]
fn the_first_paint_is_whole_and_reads_back() {
    let mut bar = Bar::new(&[(Section::Left, "12:59 am"), (Section::Right, "3:07 pm")]);
    let mut pixels = buffer();
    let mut record = Record::new(2);
    bar.paint(&mut pixels, &mut record);
    let background = rgb(bar.style.theme.background);
    let spans = bar.scene.spans().to_vec();
    assert_eq!(read(&pixels, spans[0], background), "12:59am");
    assert_eq!(read(&pixels, spans[1], background), "3:07pm");
    // Between the two: background only.
    for x in spans[0].end()..spans[1].x {
        for y in 0..HEIGHT {
            assert_eq!(pixel(&pixels, x, y), background);
        }
    }
    // The text is the foreground color, fully covered inside a segment.
    let fg = rgb(bar.style.theme.foreground);
    assert!(pixels.chunks_exact(4).any(|p| [p[2], p[1], p[0]] == fg));
}

/// One module changes and keeps its width: only its span is repainted and
/// only its span is damaged.
#[test]
fn a_change_repaints_and_damages_only_that_module() {
    let mut bar = Bar::new(&[(Section::Left, "1:00"), (Section::Right, "7:07")]);
    let mut pixels = buffer();
    let mut record = Record::new(2);
    let mut shown = Record::new(2);
    let mut spans = Vec::new();
    bar.paint(&mut pixels, &mut record);
    assert!(damage(&mut shown, &bar.scene, frame(), &mut spans));
    assert!(!bar.scene.stale(&bar.placed, &shown));
    // Scribble on the right module's pixels: a partial repaint of the left
    // one must leave them alone.
    let right = bar.scene.spans()[1];
    let marker = (right.x + 1, 1);
    let i = ((marker.1 * WIDTH + marker.0) * 4) as usize;
    pixels[i..i + 4].copy_from_slice(&[1, 2, 3, 0xff]);

    bar.set(0, "1:01", Class::Normal);
    assert!(bar.scene.stale(&bar.placed, &shown));
    bar.paint(&mut pixels, &mut record);
    assert!(!damage(&mut shown, &bar.scene, frame(), &mut spans));
    assert_eq!(spans, [bar.scene.spans()[0]]);
    let background = rgb(bar.style.theme.background);
    assert_eq!(read(&pixels, bar.scene.spans()[0], background), "1:01");
    assert_eq!(pixel(&pixels, marker.0, marker.1), [3, 2, 1]);
}

/// A width change moves spans: the whole bar is repainted and damaged.
#[test]
fn a_new_width_is_a_whole_repaint() {
    let mut bar = Bar::new(&[(Section::Center, "9:59"), (Section::Right, "1")]);
    let mut pixels = buffer();
    let mut record = Record::new(2);
    let mut shown = Record::new(2);
    let mut spans = Vec::new();
    bar.paint(&mut pixels, &mut record);
    let _ = damage(&mut shown, &bar.scene, frame(), &mut spans);
    let before = bar.scene.spans()[0];
    bar.set(0, "10:00", Class::Normal);
    bar.paint(&mut pixels, &mut record);
    assert_ne!(bar.scene.spans()[0], before);
    assert!(damage(&mut shown, &bar.scene, frame(), &mut spans));
    let background = rgb(bar.style.theme.background);
    assert_eq!(read(&pixels, bar.scene.spans()[0], background), "10:00");
    // Where the old, narrower clock was and the new one is not: clean.
    for y in 0..HEIGHT {
        assert_eq!(pixel(&pixels, before.x - 20, y), background);
    }
}

/// Two buffers: the one painted two draws ago gets both changes since.
#[test]
fn a_buffer_that_missed_a_draw_catches_up() {
    let mut bar = Bar::new(&[(Section::Left, "1"), (Section::Right, "2")]);
    let (mut a, mut b) = (buffer(), buffer());
    let (mut record_a, mut record_b) = (Record::new(2), Record::new(2));
    bar.paint(&mut a, &mut record_a);
    bar.set(0, "3", Class::Normal);
    bar.paint(&mut b, &mut record_b);
    bar.set(1, "4", Class::Normal);
    bar.paint(&mut a, &mut record_a);
    let background = rgb(bar.style.theme.background);
    let spans = bar.scene.spans().to_vec();
    assert_eq!(read(&a, spans[0], background), "3");
    assert_eq!(read(&a, spans[1], background), "4");
    // And a fresh record (a new buffer) is painted whole.
    let mut c = vec![0xaa; (WIDTH * HEIGHT * 4) as usize];
    let mut record_c = Record::new(2);
    bar.paint(&mut c, &mut record_c);
    assert_eq!(c, a);
}

#[test]
fn a_class_draws_in_its_token() {
    let mut bar = Bar::new(&[(Section::Left, "8")]);
    bar.set(0, "8", Class::Urgent);
    let mut pixels = buffer();
    let mut record = Record::new(1);
    bar.paint(&mut pixels, &mut record);
    let urgent = rgb(bar.style.theme.urgent);
    assert!(pixels.chunks_exact(4).any(|p| [p[2], p[1], p[0]] == urgent));
    let fg = rgb(bar.style.theme.foreground);
    assert!(!pixels.chunks_exact(4).any(|p| [p[2], p[1], p[0]] == fg));
}

/// No font (no modules placed): the background, and nothing else.
#[test]
fn without_a_font_only_the_background() {
    let bar = Bar::new(&[]);
    let mut pixels = vec![0x55u8; (WIDTH * HEIGHT * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, WIDTH, HEIGHT).unwrap();
    let mut record = Record::new(0);
    paint(
        &mut canvas,
        &mut record,
        &bar.scene,
        &bar.placed,
        None,
        &bar.style,
        frame(),
        &OutputView { name: None },
    );
    let background = rgb(bar.style.theme.background);
    assert!(
        pixels
            .chunks_exact(4)
            .all(|p| [p[2], p[1], p[0]] == background)
    );
}

/// Reset (a surface made again after `closed`): the next draw is whole.
#[test]
fn a_reset_record_draws_whole() {
    let mut bar = Bar::new(&[(Section::Left, "1")]);
    let mut shown = Record::new(1);
    let mut spans = Vec::new();
    bar.update();
    assert!(damage(&mut shown, &bar.scene, frame(), &mut spans));
    assert!(!damage(&mut shown, &bar.scene, frame(), &mut spans));
    assert!(spans.is_empty());
    shown.reset();
    assert!(damage(&mut shown, &bar.scene, frame(), &mut spans));
}

fn alpha(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * WIDTH + x) * 4 + 3) as usize]
}

/// A module at the bar's left edge repainted must not paint the corner
/// back in: the incremental paint equals a fresh whole one.
#[test]
fn a_repaint_at_the_edge_keeps_the_corners_cut() {
    let mut bar = Bar::new(&[(Section::Left, "12:59 am"), (Section::Right, "3:07 pm")]);
    bar.style.radius = 20;
    bar.style.opacity = 200;
    let mut record = Record::new(2);
    let mut pixels = buffer();
    bar.paint(&mut pixels, &mut record);
    assert_eq!(alpha(&pixels, 0, 0), 0, "top-left cut");
    assert_eq!(alpha(&pixels, WIDTH - 1, 0), 0, "top-right cut");
    assert_eq!(alpha(&pixels, 0, HEIGHT - 1), 0, "bottom-left cut");
    assert_eq!(alpha(&pixels, WIDTH / 2, 0), 200, "the background's alpha");
    // Change both modules (each touches an edge) and paint incrementally.
    bar.set(0, "1:00 am", Class::Warn);
    bar.set(1, "4:08 pm", Class::Urgent);
    bar.paint(&mut pixels, &mut record);
    let mut fresh_pixels = buffer();
    let mut fresh = Record::new(2);
    bar.paint(&mut fresh_pixels, &mut fresh);
    assert_eq!(pixels, fresh_pixels);
    assert_eq!(alpha(&pixels, 0, 0), 0);
    assert_eq!(alpha(&pixels, WIDTH - 1, 0), 0);
}

#[test]
fn the_radius_is_cut_back_to_what_the_bar_holds() {
    let mut bar = Bar::new(&[(Section::Left, "1")]);
    // A radius past half the height is what half the height allows.
    bar.style.radius = 500;
    let mut record = Record::new(1);
    let mut pixels = buffer();
    bar.paint(&mut pixels, &mut record);
    assert_eq!(bar.scene.corners.radius(), HEIGHT / 2);
    assert_eq!(alpha(&pixels, 0, 0), 0);
    assert!(
        alpha(&pixels, 0, HEIGHT / 2) > 240,
        "a pill's side is nearly full"
    );
    // Square by default, and a scale multiplies the radius.
    bar.style.radius = 0;
    bar.update();
    assert_eq!(bar.scene.corners.radius(), 0);
    bar.style.radius = 4;
    bar.scene.update(
        &bar.placed,
        &OutputView { name: None },
        Some(&bar.text),
        &bar.style,
        Scale::Integer(2),
        Size {
            width: WIDTH,
            height: HEIGHT,
        },
    );
    assert_eq!(bar.scene.corners.radius(), 8);
}

#[test]
fn the_style_says_when_it_needs_alpha_and_what_is_opaque() {
    let opaque = Style {
        radius: 0,
        opacity: u8::MAX,
        ..Bar::new(&[]).style
    };
    assert!(!opaque.translucent());
    assert_eq!(opaque.opaque_inset(), Some(0));
    let round = Style {
        radius: 8,
        ..opaque
    };
    assert!(round.translucent(), "corners need an alpha channel");
    assert_eq!(round.opaque_inset(), Some(9), "all but the corner squares");
    let faint = Style {
        opacity: 254,
        ..opaque
    };
    assert!(faint.translucent());
    assert_eq!(faint.opaque_inset(), None, "nothing is opaque");
    let both = Style {
        radius: 8,
        opacity: 0,
        ..opaque
    };
    assert_eq!(both.opaque_inset(), None);
}

/// An output shows the modules it lists, in its own sections: a module
/// another output shows takes no space here, its changes do not make this
/// scene stale, and the scenes of two outputs differ over the same started
/// modules.
#[test]
fn a_scene_shows_only_its_members_in_its_own_sections() {
    use super::{Member, members};
    use crate::layout::Layout;
    let mut bar = Bar::new(&[(Section::Left, "1:00"), (Section::Left, "2:00")]);
    // Started as "fixed" both; members are found by id, so build them by
    // index for two outputs: A shows both (second on the right), B only the
    // second, on the left.
    let a = [
        Member {
            module: 0,
            section: Section::Left,
            margin: 0,
        },
        Member {
            module: 1,
            section: Section::Right,
            margin: 0,
        },
    ];
    let b = [Member {
        module: 1,
        section: Section::Left,
        margin: 0,
    }];
    let mut scene_a = Scene::with_members(&a);
    let mut scene_b = Scene::with_members(&b);
    let output = OutputView { name: None };
    for (scene, spans) in [(&mut scene_a, 2), (&mut scene_b, 1)] {
        scene.update(
            &bar.placed,
            &output,
            Some(&bar.text),
            &bar.style,
            Scale::Integer(1),
            Size {
                width: WIDTH,
                height: HEIGHT,
            },
        );
        assert_eq!(scene.spans().len(), spans);
    }
    assert_eq!(scene_a.spans()[0].x, 0);
    assert_eq!(scene_a.spans()[1].end(), WIDTH);
    assert_eq!(scene_b.spans()[0].x, 0);
    assert_eq!(scene_b.module(0), Some(1));
    assert_eq!(scene_a.section_of(1), Some(Section::Right));
    assert_eq!(scene_b.section_of(0), None);
    // Module 0 changes: A is stale, B (which does not show it) is not.
    let shown_a = Record::new(2);
    let mut damaged = Vec::new();
    let mut shown_a = shown_a;
    let mut shown_b = Record::new(1);
    let _ = super::damage(&mut shown_a, &scene_a, frame(), &mut damaged);
    let _ = super::damage(&mut shown_b, &scene_b, frame(), &mut damaged);
    assert!(!scene_a.stale(&bar.placed, &shown_a));
    bar.set(0, "1:01", Class::Normal);
    assert!(scene_a.stale(&bar.placed, &shown_a));
    assert!(!scene_b.stale(&bar.placed, &shown_b));
    // Members resolve from a layout by id, skipping what did not start.
    let layout = Layout {
        left: vec!["fixed", "gone"],
        center: Vec::new(),
        right: Vec::new(),
        ..Layout::default()
    };
    assert_eq!(
        members(&layout, &bar.placed),
        [Member {
            module: 0,
            section: Section::Left,
            margin: 0,
        }]
    );
    // An empty scene is fine: nothing to lay out, nothing stale.
    let empty = Scene::with_members(&[]);
    assert!(!empty.stale(&bar.placed, &Record::new(0)));
}
