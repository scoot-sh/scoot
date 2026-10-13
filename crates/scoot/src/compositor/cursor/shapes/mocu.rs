//! Reads the vendored Mocu cursor SVGs (`mocu/*.svg`, CC0 1.0 -- see
//! `mocu/README.md`) into the outlines scoot rasterises.
//!
//! The files are the single source of truth: they are compiled in with
//! `include_str!` and read here, so the art scoot draws is the art in the
//! files. Mocu authors every cursor the same way, and that is all this
//! reads:
//!
//! - `<defs>` holds the outlines, as `<path id d>` or `<circle id cx cy r>`.
//! - Each `<use href="#id">` whose style sets a `stroke-width` (and no
//!   `opacity`) is one painted layer: the outline stroked with the stroke
//!   color at that width, then filled with the fill color. Layers paint in
//!   file order. The faint `opacity:.1` uses (and the `<g>` of them) are the
//!   drop shadow, which [`super::generate`] derives from the layers.
//! - A `<circle mask=...>` is the busy spinner: that circle in the stroke
//!   color, seen through the file's `<mask>`, whose children (a fan of
//!   rotated grey wedges, a white dot, a black hub) give each point its
//!   opacity -- the last child covering a point decides it, as SVG paints.
//!   The animation's first frame is what is drawn.
//! - `<circle id="hot">` is the hotspot.
//!
//! Mocu's left-handed `class="left(...)"` transforms are ignored: scoot
//! draws the right-handed set. Anything the reader does not understand is
//! skipped, never a panic; the tests assert every vendored file yields its
//! layers and hotspot.

use super::svgpath::{self, Subpath};

/// Every vendored Mocu file scoot draws, by Mocu's own cursor name, in
/// [`super::Shape::ALL`] order.
pub(crate) const FILES: [(&str, &str); 34] = [
    ("default", include_str!("mocu/default.svg")),
    ("context-menu", include_str!("mocu/context-menu.svg")),
    ("help", include_str!("mocu/help.svg")),
    ("pointer", include_str!("mocu/pointer.svg")),
    ("progress", include_str!("mocu/progress.svg")),
    ("wait", include_str!("mocu/wait.svg")),
    ("cell", include_str!("mocu/cell.svg")),
    ("crosshair", include_str!("mocu/crosshair.svg")),
    ("text", include_str!("mocu/text.svg")),
    ("vertical-text", include_str!("mocu/vertical-text.svg")),
    ("alias", include_str!("mocu/alias.svg")),
    ("copy", include_str!("mocu/copy.svg")),
    ("no-drop", include_str!("mocu/no-drop.svg")),
    ("not-allowed", include_str!("mocu/not-allowed.svg")),
    ("grab", include_str!("mocu/grab.svg")),
    ("grabbing", include_str!("mocu/grabbing.svg")),
    ("e-resize", include_str!("mocu/e-resize.svg")),
    ("n-resize", include_str!("mocu/n-resize.svg")),
    ("ne-resize", include_str!("mocu/ne-resize.svg")),
    ("nw-resize", include_str!("mocu/nw-resize.svg")),
    ("s-resize", include_str!("mocu/s-resize.svg")),
    ("se-resize", include_str!("mocu/se-resize.svg")),
    ("sw-resize", include_str!("mocu/sw-resize.svg")),
    ("w-resize", include_str!("mocu/w-resize.svg")),
    ("ew-resize", include_str!("mocu/ew-resize.svg")),
    ("ns-resize", include_str!("mocu/ns-resize.svg")),
    ("nesw-resize", include_str!("mocu/nesw-resize.svg")),
    ("nwse-resize", include_str!("mocu/nwse-resize.svg")),
    ("col-resize", include_str!("mocu/col-resize.svg")),
    ("row-resize", include_str!("mocu/row-resize.svg")),
    ("all-scroll", include_str!("mocu/all-scroll.svg")),
    ("zoom-in", include_str!("mocu/zoom-in.svg")),
    ("zoom-out", include_str!("mocu/zoom-out.svg")),
    ("dnd-ask", include_str!("mocu/dnd-ask.svg")),
];

/// One painted layer: an outline (grid units) and its stroke width (units).
#[derive(Clone, Debug)]
pub(crate) struct Layer {
    pub subs: Vec<Subpath>,
    pub stroke: f64,
}

/// The busy spinner: a disk in the stroke color, its opacity per point
/// given by `mask` (later entries win, as SVG paints them).
#[derive(Clone, Debug)]
pub(crate) struct Spinner {
    pub disk: Subpath,
    pub mask: Vec<(Vec<Subpath>, f64)>,
}

/// One cursor's art, in Mocu's 24-unit grid.
#[derive(Clone, Debug)]
pub(crate) struct Art {
    pub layers: Vec<Layer>,
    pub spinner: Option<Spinner>,
    pub hot: (f64, f64),
}

/// One `<tag attr="value" ...>` of a file.
struct Element<'a> {
    tag: &'a str,
    attrs: Vec<(&'a str, &'a str)>,
    closing: bool,
    empty: bool,
}

impl<'a> Element<'a> {
    fn get(&self, name: &str) -> Option<&'a str> {
        self.attrs.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }

    fn num(&self, name: &str) -> f64 {
        self.get(name).and_then(|v| v.parse().ok()).unwrap_or(0.0)
    }

    /// A style property, e.g. `stroke-width` in `fill:#1a1b1c;stroke-width:2`.
    fn style(&self, prop: &str) -> Option<&'a str> {
        self.get("style")?.split(';').find_map(|decl| {
            let (k, v) = decl.split_once(':')?;
            (k.trim() == prop).then_some(v.trim())
        })
    }
}

/// The file's elements, in order. The XML declaration, comments and text
/// between tags are skipped.
fn elements(svg: &str) -> Vec<Element<'_>> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('>') else { break };
        let body = &rest[..end];
        rest = &rest[end + 1..];
        if body.starts_with('?') || body.starts_with('!') {
            continue;
        }
        let closing = body.starts_with('/');
        let empty = body.ends_with('/');
        let body = body.trim_start_matches('/').trim_end_matches('/');
        let tag_end = body
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(body.len());
        let mut attrs = Vec::new();
        let mut a = &body[tag_end..];
        while let Some(eq) = a.find("=\"") {
            let name = a[..eq].trim();
            let after = &a[eq + 2..];
            let Some(close) = after.find('"') else { break };
            attrs.push((name, &after[..close]));
            a = &after[close + 1..];
        }
        out.push(Element {
            tag: &body[..tag_end],
            attrs,
            closing,
            empty,
        });
    }
    out
}

/// A `#rgb`/`#rrggbb` grey's luminance, `0.0..=1.0` (mask contents are
/// greys). No color is SVG's default black.
fn luminance(color: Option<&str>) -> f64 {
    let Some(hex) = color.and_then(|c| c.strip_prefix('#')) else {
        return 0.0;
    };
    let channel = match hex.len() {
        3 => u8::from_str_radix(&hex[..1], 16).map(|v| v * 17),
        6 => u8::from_str_radix(&hex[..2], 16),
        _ => return 0.0,
    };
    channel.map_or(0.0, |v| f64::from(v) / 255.0)
}

/// A full circle as one subpath, flattened to within `tol` units.
pub(crate) fn circle(cx: f64, cy: f64, r: f64, tol: f64) -> Subpath {
    let step = if tol >= r {
        std::f64::consts::FRAC_PI_2
    } else {
        2.0 * (1.0 - tol / r).acos()
    };
    let n = ((std::f64::consts::TAU / step.max(1e-3)).ceil() as usize).clamp(8, 512);
    (0..n)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            (cx + r * t.cos(), cy + r * t.sin())
        })
        .collect()
}

/// An element's geometry -- a path's `d` or a circle -- with its
/// `rotate(a,cx,cy)` transform applied if it has one.
fn geometry(el: &Element<'_>, tol: f64) -> Option<Vec<Subpath>> {
    let mut subs = match el.tag {
        "path" => svgpath::parse(el.get("d")?, tol),
        "circle" => vec![circle(el.num("cx"), el.num("cy"), el.num("r"), tol)],
        _ => return None,
    };
    let rotate = el
        .get("transform")
        .and_then(|t| t.strip_prefix("rotate("))
        .and_then(|t| t.strip_suffix(')'));
    if let Some(args) = rotate {
        let v: Vec<f64> = args
            .split(',')
            .filter_map(|n| n.trim().parse().ok())
            .collect();
        if let [deg, cx, cy] = v[..] {
            let (s, c) = deg.to_radians().sin_cos();
            for p in subs.iter_mut().flatten() {
                let (dx, dy) = (p.0 - cx, p.1 - cy);
                *p = (cx + dx * c - dy * s, cy + dx * s + dy * c);
            }
        }
    }
    Some(subs)
}

/// Reads one Mocu file, curves flattened to within `tol` grid units.
pub(crate) fn read(svg: &str, tol: f64) -> Art {
    let mut defs: Vec<(&str, Vec<Subpath>)> = Vec::new();
    let mut mask = Vec::new();
    let mut art = Art {
        layers: Vec::new(),
        spinner: None,
        hot: (0.0, 0.0),
    };
    let mut disk = None;
    let (mut in_defs, mut in_mask, mut in_shadow) = (false, false, false);
    for el in elements(svg) {
        let open = !el.closing && !el.empty;
        match el.tag {
            "defs" => in_defs = open,
            "mask" => in_mask = open,
            "g" => in_shadow = open && el.style("opacity").is_some(),
            _ if el.closing => {}
            _ if in_mask => {
                if let Some(subs) = geometry(&el, tol) {
                    let lum = luminance(el.get("fill").or_else(|| el.style("fill")));
                    mask.push((subs, lum));
                }
            }
            _ if in_defs => {
                if let (Some(id), Some(subs)) = (el.get("id"), geometry(&el, tol)) {
                    defs.push((id, subs));
                }
            }
            "use" if !in_shadow && el.style("opacity").is_none() => {
                let stroke = el.style("stroke-width").and_then(|w| w.parse().ok());
                let id = el
                    .get("xlink:href")
                    .or_else(|| el.get("href"))
                    .and_then(|h| h.strip_prefix('#'));
                let found = id.and_then(|id| defs.iter().find(|(d, _)| *d == id));
                if let (Some(stroke), Some((_, subs))) = (stroke, found) {
                    art.layers.push(Layer {
                        subs: subs.clone(),
                        stroke,
                    });
                }
            }
            "circle" if el.get("id") == Some("hot") => art.hot = (el.num("cx"), el.num("cy")),
            "circle" if el.get("mask").is_some() => {
                disk = Some(circle(el.num("cx"), el.num("cy"), el.num("r"), tol));
            }
            _ => {}
        }
    }
    art.spinner = disk.map(|disk| Spinner { disk, mask });
    art
}
