use super::{MAX_BYTES, MAX_COMMANDS, MAX_COORD, MAX_SEGS, Seg, Vector, ViewBox, parse};
use crate::icon::raster::Rasterizer;

/// Real-shaped icon data: Material Design's home, circle-ring, cloud and
/// grid, plus quadratics, arcs and the shorthand forms.
const CORPUS: &[&str] = &[
    "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z",
    "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8z",
    "M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96z",
    "M3 3h8v8H3zM13 3h8v8h-8zM3 13h8v8H3zM13 13h8v8h-8z",
    "M2 12q5-10 10 0t10 0",
    "M2 12Q7 2 12 12T22 12",
    "m0 0 10 0 0 10-10 0z",
    "M4 4A4 4 0 0 1 12 4 4 4 0 0 1 20 4M4 12a4 6 30 1 0 8 0",
    "M1,1 L2,2 3,3 z",
    "M0 0h.5.5v.5",
];

fn viewbox() -> ViewBox {
    ViewBox::default()
}

/// The path as text again, in absolute commands only.
fn to_d(segs: &[Seg]) -> String {
    let mut d = String::new();
    for seg in segs {
        match *seg {
            Seg::Move((x, y)) => d.push_str(&format!("M{x} {y}")),
            Seg::Line((x, y)) => d.push_str(&format!("L{x} {y}")),
            Seg::Cubic((a, b), (c, e), (x, y)) => {
                d.push_str(&format!("C{a} {b} {c} {e} {x} {y}"));
            }
            Seg::Close => d.push('Z'),
        }
    }
    d
}

#[test]
fn a_simple_path_becomes_absolute_segments() {
    let segs = parse("M0 0L10 0 10 10z").unwrap();
    assert_eq!(
        segs,
        [
            Seg::Move((0.0, 0.0)),
            Seg::Line((10.0, 0.0)),
            Seg::Line((10.0, 10.0)),
            Seg::Close
        ]
    );
}

#[test]
fn relative_commands_add_to_the_pen_and_h_and_v_keep_the_other_axis() {
    let segs = parse("m1 1l2 0h3v4z").unwrap();
    assert_eq!(
        segs,
        [
            Seg::Move((1.0, 1.0)),
            Seg::Line((3.0, 1.0)),
            Seg::Line((6.0, 1.0)),
            Seg::Line((6.0, 5.0)),
            Seg::Close
        ]
    );
    // A relative move after the close starts from the subpath's start.
    let segs = parse("M2 2 h4 z m1 1 l1 0").unwrap();
    assert!(segs.contains(&Seg::Move((3.0, 3.0))), "{segs:?}");
}

#[test]
fn the_extra_pairs_of_a_move_are_lines_and_a_command_repeats() {
    assert_eq!(
        parse("M1 2 3 4").unwrap(),
        [Seg::Move((1.0, 2.0)), Seg::Line((3.0, 4.0))]
    );
    assert_eq!(
        parse("m1 2 3 4").unwrap(),
        [Seg::Move((1.0, 2.0)), Seg::Line((4.0, 6.0))]
    );
    assert_eq!(
        parse("M0 0 H1 2 3").unwrap(),
        [
            Seg::Move((0.0, 0.0)),
            Seg::Line((1.0, 0.0)),
            Seg::Line((2.0, 0.0)),
            Seg::Line((3.0, 0.0))
        ]
    );
}

#[test]
fn numbers_may_touch_and_take_exponents() {
    // `.5.5` is two numbers, `1-1` too, and a comma may stand between.
    assert_eq!(
        parse("M.5.5L1-1").unwrap(),
        [Seg::Move((0.5, 0.5)), Seg::Line((1.0, -1.0))]
    );
    assert_eq!(
        parse("M1e1 2E0 L-.5,+.25").unwrap(),
        [Seg::Move((10.0, 2.0)), Seg::Line((-0.5, 0.25))]
    );
    assert_eq!(
        parse("  M\t1\n2\r\nL3 3  ").unwrap(),
        [Seg::Move((1.0, 2.0)), Seg::Line((3.0, 3.0))]
    );
}

#[test]
fn a_smooth_command_reflects_the_previous_control_point() {
    let segs = parse("M0 0 C1 2 3 4 5 5 S9 9 10 10").unwrap();
    // Reflection of (3, 4) through (5, 5) is (7, 6).
    assert_eq!(
        segs[2],
        Seg::Cubic((7.0, 6.0), (9.0, 9.0), (10.0, 10.0)),
        "{segs:?}"
    );
    // After a line there is nothing to reflect: the first control is the pen.
    let segs = parse("M0 0 L5 5 S9 9 10 10").unwrap();
    assert_eq!(segs[2], Seg::Cubic((5.0, 5.0), (9.0, 9.0), (10.0, 10.0)));
    // A quadratic and its smooth continuation, as cubics (the 2/3 rule).
    let segs = parse("M0 0 Q3 3 6 0 T12 0").unwrap();
    assert_eq!(
        segs[1],
        Seg::Cubic((2.0, 2.0), (4.0, 2.0), (6.0, 0.0)),
        "{segs:?}"
    );
    // T reflects (3, 3) through (6, 0): the quadratic control (9, -3).
    assert_eq!(
        segs[2],
        Seg::Cubic((8.0, -2.0), (10.0, -2.0), (12.0, 0.0)),
        "{segs:?}"
    );
}

#[test]
fn arcs_are_cubics_that_start_and_end_where_the_path_says() {
    // A semicircle of radius 12 from (0, 12) to (24, 12): two quarter
    // cubics, the last ending exactly at the endpoint.
    let segs = parse("M0 12a12 12 0 1 0 24 0").unwrap();
    assert_eq!(segs.len(), 3, "{segs:?}");
    let Seg::Cubic(_, _, mid) = segs[1] else {
        panic!("{segs:?}")
    };
    // The midpoint is on the circle around (12, 12).
    let radius = ((mid.0 - 12.0).powi(2) + (mid.1 - 12.0).powi(2)).sqrt();
    assert!((radius - 12.0).abs() < 0.01, "{mid:?} {radius}");
    assert!(matches!(segs[2], Seg::Cubic(_, _, (24.0, 12.0))));
    // The compact flags form: `00.5.5` is flags 0 and 0, then .5 and .5.
    let segs = parse("M0 0a1 1 0 00.5.5").unwrap();
    assert!(matches!(segs.last(), Some(Seg::Cubic(_, _, (x, y))) if *x == 0.5 && *y == 0.5));
    // A zero radius is a line, the same endpoints draw nothing, and radii
    // too small are scaled up to reach.
    assert_eq!(
        parse("M0 0A0 5 0 0 0 4 4").unwrap(),
        [Seg::Move((0.0, 0.0)), Seg::Line((4.0, 4.0))]
    );
    assert!(parse("M0 0 A5 5 0 0 0 0 0").is_err(), "draws nothing");
    let segs = parse("M0 0A0.1 0.1 0 0 1 10 0").unwrap();
    assert!(matches!(segs.last(), Some(Seg::Cubic(_, _, (10.0, 0.0)))));
}

#[test]
fn every_way_to_be_malformed_is_a_refusal_naming_a_byte() {
    for bad in [
        "",
        "   ",
        "L1 1",
        "1 2",
        "M",
        "M1",
        "M1 2 L",
        "M1 2 X 3 4",
        "M1 2 3",
        "M1 2 L 1e",
        "M1 2 L 1e+",
        "M1 2 L - 1",
        "M1 2 A1 1 0 2 0 1 1",
        "M1 2 A1 1 0 0 x 1 1",
        "M1 2 A1 1 0 0 0 1",
        "M 1,,2",
        "M 1 2,, L 3 4",
        "M1 2 , L 3 4",
        "M0 0 Z 5",
        "M1 2 z",
        "M1 2 M3 4",
        "M0 0 L1e999 1",
        "M0 0 L1000001 0",
        "M0 0 L1 1 é",
        "M0 0 L1 1 \0",
        "M0 0 L1 1 # comment",
        "M0 0 L NaN 1",
        "M0 0 L inf 1",
        "M0 0 L 0x10 1",
        "M0 0 L 1_0 1",
    ] {
        let error = parse(bad).unwrap_err();
        assert!(error.at <= bad.len(), "{bad:?}: {error}");
        assert!(!error.what.is_empty());
        assert!(error.to_string().starts_with("at byte "), "{error}");
    }
}

#[test]
fn a_path_must_start_with_a_move_and_draw_something() {
    let error = parse("L0 0 10 10").unwrap_err();
    assert!(error.what.contains("M or m"), "{error}");
    let error = parse("M5 5").unwrap_err();
    assert!(error.what.contains("draws nothing"), "{error}");
}

#[test]
fn a_path_past_the_bounds_is_refused() {
    // Too many bytes.
    let long = format!("M0 0{}", " ".repeat(MAX_BYTES));
    assert!(parse(&long).unwrap_err().what.contains("too long"));
    // Exactly at the command bound (the move and 1023 lines) is fine; one
    // more is refused.
    let at = format!("M0 0{}", "l1 1".repeat(MAX_COMMANDS - 1));
    assert!(at.len() <= MAX_BYTES);
    assert_eq!(parse(&at).unwrap().len(), MAX_COMMANDS);
    let over = format!("M0 0{}", "l1 1".repeat(MAX_COMMANDS));
    assert!(parse(&over).unwrap_err().what.contains("commands"));
    // Implicit repeats count each time.
    let over = format!("M0 0 l{}", "1 1 ".repeat(MAX_COMMANDS));
    assert!(parse(&over).unwrap_err().what.contains("commands"));
    // The most segments a path can make (every command a near-full arc,
    // four cubics) stays within the segment bound.
    let arcs = format!("M0 0{}", "a1 1 0 11.001 0".repeat(MAX_COMMANDS - 1));
    assert!(arcs.len() <= MAX_BYTES);
    let segs = parse(&arcs).unwrap();
    assert!(segs.len() <= MAX_SEGS, "{}", segs.len());
    // Numbers at the limit are taken, past it refused.
    assert!(parse(&format!("M{MAX_COORD} -{MAX_COORD} L0 0")).is_ok());
    assert!(parse("M1000000.5 0 L0 0").is_err());
}

#[test]
fn relative_moves_that_add_up_past_the_bound_are_refused() {
    let mut d = String::from("M0 0");
    for _ in 0..8 {
        d.push_str(" l1000000 0");
    }
    assert!(parse(&d).unwrap_err().what.contains("out of range"));
}

#[test]
fn a_valid_path_written_out_absolute_parses_to_the_same_segments() {
    for &source in CORPUS {
        let segs = parse(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        let text = to_d(&segs);
        let again = parse(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(segs, again, "{source}");
    }
}

#[test]
fn viewboxes_parse_and_refuse() {
    assert_eq!(ViewBox::parse("0 0 24 24").unwrap(), ViewBox::default());
    let v = ViewBox::parse("-1, -2 ,448 512").unwrap();
    assert_eq!((v.x, v.y, v.width, v.height), (-1.0, -2.0, 448.0, 512.0));
    for bad in [
        "",
        "0 0 24",
        "0 0 24 24 24",
        "0 0 0 24",
        "0 0 24 -1",
        "0 0 0.0001 24",
        "a b c d",
        "0 0 1e999 1",
        &"1 ".repeat(100),
    ] {
        assert!(ViewBox::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn equal_when_made_from_the_same_text_and_viewbox() {
    let a = Vector::parse("M0 0h1v1z", viewbox()).unwrap();
    let b = Vector::parse("M0 0h1v1z", viewbox()).unwrap();
    let c = Vector::parse("M0 0h1v2z", viewbox()).unwrap();
    let d = Vector::parse(
        "M0 0h1v1z",
        ViewBox {
            width: 48.0,
            ..viewbox()
        },
    )
    .unwrap();
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, d);
    // Each parse has its own cache id, equal or not.
    assert_ne!(a.id(), b.id());
}

/// xorshift64*: a deterministic stream, so a failure replays.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// What a parse that succeeded must satisfy; drawing it must not panic.
fn check_ok(segs: &[Seg], raster: &mut Rasterizer) {
    let bound = MAX_COORD * 4.0;
    let ok =
        |(x, y): (f32, f32)| x.is_finite() && y.is_finite() && x.abs() <= bound && y.abs() <= bound;
    for seg in segs {
        let good = match *seg {
            Seg::Move(p) | Seg::Line(p) => ok(p),
            Seg::Cubic(a, b, c) => ok(a) && ok(b) && ok(c),
            Seg::Close => true,
        };
        assert!(good, "{seg:?}");
    }
    assert!(segs.len() <= MAX_SEGS);
    // The absolute text of what parsed parses too (or is refused for
    // size), and fills.
    if let Ok(vector) = Vector::parse(&to_d(segs), viewbox()) {
        let mut out = vec![0u8; 20 * 20];
        raster.fill(&vector, 20, &mut out);
    }
}

#[test]
fn fuzzed_path_strings_never_panic() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut raster = Rasterizer::default();
    let alphabet = b"MmLlHhVvCcSsQqTtAaZz0123456789.,+-eE \n\t";
    let mut accepted = 0;
    let mut try_one = |bytes: &[u8], raster: &mut Rasterizer| {
        if let Ok(text) = std::str::from_utf8(bytes) {
            if let Ok(segs) = parse(text) {
                accepted += 1;
                check_ok(&segs, raster);
            }
        }
    };
    // Random strings over the grammar's alphabet.
    for _ in 0..20_000 {
        let len = rng.below(120);
        let mut s: Vec<u8> = (0..len)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        if rng.below(2) == 0 {
            s.insert(0, b'M');
        }
        try_one(&s, &mut raster);
    }
    // Mutations of real paths: every truncation, then random byte edits.
    for &source in CORPUS {
        for end in 0..=source.len() {
            try_one(&source.as_bytes()[..end], &mut raster);
        }
        for _ in 0..2_000 {
            let mut s = source.as_bytes().to_vec();
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(s.len() + 1);
                match rng.below(3) {
                    0 if at < s.len() => s[at] = alphabet[rng.below(alphabet.len())],
                    1 => s.insert(at, alphabet[rng.below(alphabet.len())]),
                    _ if at < s.len() => {
                        s.remove(at);
                    }
                    _ => {}
                }
            }
            try_one(&s, &mut raster);
        }
    }
    // The stream is not all refusals, or it would test nothing past the
    // first byte.
    assert!(accepted > 1_000, "only {accepted} accepted");
}

#[test]
fn hostile_numbers_and_shapes_parse_or_refuse_and_draw_without_panic() {
    let mut raster = Rasterizer::default();
    for d in [
        "M1000000 1000000 L-1000000 -1000000 L1000000 -1000000z",
        "M0 0 C1000000 1000000 -1000000 1000000 1000000 -1000000z",
        "M0 0 A1000000 0.000001 90 1 1 0.0001 0.0001",
        "M0 0 A0.000001 1000000 45 0 1 1000000 1000000",
        "M0 0 a1e-30 1e-30 0 0 0 1e-30 1e-30",
        "M0 0 L1e-45 1e-45 L0 1e-40z",
        "M0 0 Q1000000 0 0 1000000 T-1000000 0",
        "M-1e6 -1e6 h2e6 v2e6 h-2e6z",
        "M0 0 a1 1 -720 1 1 2 0 a1 1 720 0 0 -2 0",
    ] {
        if let Ok(vector) = Vector::parse(d, viewbox()) {
            for side in [1u32, 7, 24, 100] {
                let mut out = vec![0u8; (side * side) as usize];
                raster.fill(&vector, side, &mut out);
            }
        }
    }
    // A tiny viewbox scales a normal path enormously.
    let tiny = ViewBox::parse("0 0 0.001 0.001").unwrap();
    let vector = Vector::parse("M0 0 h1 v1 h-1z", tiny).unwrap();
    let mut out = vec![0u8; 32 * 32];
    raster.fill(&vector, 32, &mut out);
}
