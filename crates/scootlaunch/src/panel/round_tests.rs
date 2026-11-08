//! The search field's focus ring and the selected row's inverted match
//! chip share the selection highlight's corner radius, and the chip stays
//! inside the highlight at every scale.

use scootui::density::Scale;

use super::bytes_tests::{entries, test_text};
use super::*;
use crate::rank::frecency::Frecency;

const SCALES: [Scale; 8] = [
    Scale::Integer(1),
    Scale::Fractional(120),
    Scale::Fractional(150),
    Scale::Fractional(180),
    Scale::Integer(2),
    Scale::Fractional(225),
    Scale::Fractional(270),
    Scale::Integer(3),
];

/// The apps panel for `ui` at `scale`, painted whole: the BGRA bytes, the
/// buffer's width, and the layout it was drawn with.
fn paint(look: &Look, ui: &Ui, scale: Scale, rows: u32) -> (Vec<u8>, u32, Layout) {
    let dir = tempfile::tempdir().expect("tmp");
    let mut text = test_text(dir.path());
    let mut icons = crate::icons::Cache::default();
    let logical = Layout::new(look.font_size_px, look.radius, Scale::Integer(1));
    let width = (30.0 * look.font_size_px) as u32;
    let height = logical.height(rows);
    let (width_d, height_d) = (scale_len(width, scale), scale_len(height, scale));
    let mut pixels = vec![0u8; width_d as usize * height_d as usize * 4];
    let view = PanelView {
        ui,
        look,
        width,
        height,
        rows,
        shown: rows,
        error: None,
        empty_text: "empty",
        scale,
        icon_theme: "Fixture",
        icon_bases: &[],
    };
    paint_panel(
        &mut pixels,
        (width_d, height_d),
        &view,
        &mut text,
        &mut icons,
        None,
    );
    (pixels, width_d, device_layout(look, scale, height, rows))
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = (y as usize * width as usize + x as usize) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

/// A list whose one row is `name`, with `query` typed and row 0 selected.
fn ui_with(name: &str, query: &str) -> Ui {
    let mut list = entries(1);
    list[0].name = name.to_owned();
    let mut ui = Ui::open(list, Frecency::empty(), 0, 10);
    for ch in query.chars() {
        let mut utf8 = [0u8; 4];
        ui.key(ch as u32, false, ch.encode_utf8(&mut utf8));
    }
    ui
}

/// The looks the radius ranges over: the default, a pill (cut back to
/// half a row), and the smallest the highlight rounds at all.
fn looks() -> Vec<Look> {
    [12, 24, 64, 4, 1]
        .into_iter()
        .map(|radius| Look {
            radius,
            ..Look::default()
        })
        .collect()
}

#[test]
fn the_field_and_the_list_take_the_highlights_radius() {
    for look in looks() {
        for scale in SCALES {
            let logical = Layout::new(look.font_size_px, look.radius, Scale::Integer(1));
            let layout = device_layout(&look, scale, logical.height(6), 6);
            assert_eq!(layout.field_geometry(400).radius, layout.radius);
            assert_eq!(layout.list_geometry(400).radius, layout.radius);
        }
    }
}

#[test]
fn the_search_ring_is_rounded_where_the_selection_pill_is() {
    let look = Look::default();
    let ui = ui_with("Firefox", "");
    for scale in SCALES {
        let (pixels, width, layout) = paint(&look, &ui, scale, 6);
        let field = layout.field(width);
        assert!(layout.radius > 2, "{scale:?}: the radius under test");
        // The ring's corner pixel is cut (the panel surface shows), its
        // top and left sides are the accent.
        let accent = look.theme.accent;
        let ring = [accent.b, accent.g, accent.r, 255];
        assert_ne!(pixel(&pixels, width, field.x, field.y), ring, "{scale:?}");
        assert_eq!(
            pixel(&pixels, width, field.x + field.width / 2, field.y),
            ring,
            "{scale:?}"
        );
        assert_eq!(
            pixel(&pixels, width, field.x, field.y + field.height / 2),
            ring,
            "{scale:?}"
        );
    }
}

/// A pill pixel this covered counts as on the pill. The rounded chip's own
/// rim shares the pill's antialiased edge pixels at a few percent; a square
/// chip corner lands on pixels the pill covers 70% or less.
const ON_PILL: u8 = 224;

/// The geometric pin: with a chip running to the row's text end, no pixel
/// the selection pill does not (nearly) cover changes when the query adds the
/// chip, at every scale and radius. The pill's coverage is its own paint on
/// an empty canvas; a square chip corner outside the pill moves a pixel
/// that coverage says is not fully the pill's.
#[test]
fn the_match_chip_stays_inside_the_pill_at_every_scale() {
    let name = "w".repeat(90);
    let query = "w".repeat(70);
    for look in looks() {
        assert!(matches!(
            look.theme.match_on_selection(),
            scootui::theme::SelectedMatch::Inverted
        ));
        for scale in SCALES {
            let (plain, width, layout) = paint(&look, &ui_with(&name, ""), scale, 6);
            let (chipped, ..) = paint(&look, &ui_with(&name, &query), scale, 6);
            let row = layout.row(width, 0);
            let mut mask = vec![0u8; plain.len()];
            let height = (plain.len() / 4) as u32 / width;
            let mut canvas = Canvas::new(&mut mask, width, height).expect("canvas");
            crate::highlight::fill_round_rect(&mut canvas, row, layout.radius, look.theme.accent);
            let mut moved = 0;
            for y in row.y..row.y + row.height {
                for x in row.x..row.right() {
                    let changed = pixel(&plain, width, x, y) != pixel(&chipped, width, x, y);
                    assert!(
                        !changed || pixel(&mask, width, x, y)[3] >= ON_PILL,
                        "radius {} {scale:?}: ({x}, {y}) changed outside the pill",
                        look.radius
                    );
                    moved += usize::from(changed);
                }
            }
            assert!(moved > 0, "radius {} {scale:?}: no chip drawn", look.radius);
        }
    }
}
