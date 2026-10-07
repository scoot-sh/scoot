//! Tests for the transition math: each kind at `t = 0`, mid and `1`,
//! easing curves, angle wipes, damage rectangles (including the property
//! that every changed pixel is damaged), mid-transition restart
//! continuity, hostile input, and the degrade path.

use super::*;

fn spec(kind: Kind) -> Spec {
    Spec {
        kind,
        duration_ms: 500,
        easing: Easing::Linear,
        angle_deg: 0.0,
        pos: (0.5, 0.5),
    }
}

fn all_pixels(dims: (u32, u32), mut f: impl FnMut(u32, u32)) {
    for y in 0..dims.1 {
        for x in 0..dims.0 {
            f(x, y);
        }
    }
}

#[test]
fn kind_names_round_trip() {
    for kind in Kind::ALL {
        assert_eq!(Kind::from_name(kind.name()), Some(kind));
        assert_eq!(Kind::parse(kind.name()), Ok(kind));
    }
    assert_eq!(
        Kind::parse("dissolve"),
        Err(ParseError::UnknownKind("dissolve".to_owned()))
    );
    assert_eq!(
        Kind::parse("FADE"),
        Err(ParseError::UnknownKind("FADE".to_owned()))
    );
    assert_eq!(Kind::parse(""), Err(ParseError::UnknownKind(String::new())));
}

#[test]
fn easing_names_round_trip() {
    for easing in Easing::ALL {
        assert_eq!(Easing::from_name(easing.name()), Some(easing));
        assert_eq!(Easing::parse(easing.name()), Ok(easing));
    }
    assert_eq!(
        Easing::parse("bounce"),
        Err(ParseError::UnknownEasing("bounce".to_owned()))
    );
}

#[test]
fn durations() {
    assert_eq!(parse_duration_ms("0"), Ok(0));
    assert_eq!(parse_duration_ms("500"), Ok(500));
    assert_eq!(parse_duration_ms("60000"), Ok(60_000));
    for bad in [
        "",
        " 500",
        "500 ",
        "+500",
        "-1",
        "0.5",
        "60001",
        "99999999999999999999999",
        "half a second",
    ] {
        assert_eq!(
            parse_duration_ms(bad),
            Err(ParseError::BadDuration(bad.to_owned())),
            "{bad:?}"
        );
    }
}

#[test]
fn angles_normalize() {
    assert_eq!(parse_angle_deg("0"), Ok(0.0));
    assert_eq!(parse_angle_deg("45"), Ok(45.0));
    assert_eq!(parse_angle_deg("720"), Ok(0.0));
    assert_eq!(parse_angle_deg("-90"), Ok(270.0));
    assert_eq!(parse_angle_deg("359.75"), Ok(359.75));
    assert_eq!(parse_angle_deg("-0.0"), Ok(0.0));
    for bad in ["", "NaN", "inf", "-inf", "ninety", "45 ", " 45", "45deg"] {
        assert_eq!(
            parse_angle_deg(bad),
            Err(ParseError::BadAngle(bad.to_owned())),
            "{bad:?}"
        );
    }
}

#[test]
fn positions() {
    assert_eq!(parse_position("0.5,0.5"), Ok((0.5, 0.5)));
    assert_eq!(parse_position("0,0"), Ok((0.0, 0.0)));
    assert_eq!(parse_position("1,1"), Ok((1.0, 1.0)));
    for bad in [
        "",
        "0.5",
        "0.5,0.5,0.5",
        "0.5;0.5",
        "-0.1,0.5",
        "0.5,1.1",
        "NaN,0.5",
        "0.5,inf",
        "0.5, 0.5",
        "left,top",
    ] {
        assert_eq!(
            parse_position(bad),
            Err(ParseError::BadPosition(bad.to_owned())),
            "{bad:?}"
        );
    }
}

#[test]
fn easing_endpoints_are_exact() {
    for easing in Easing::ALL {
        assert_eq!(ease(easing, 0.0), 0.0, "{easing:?}");
        assert_eq!(ease(easing, 1.0), 1.0, "{easing:?}");
        // A clock that overshoots still ends exactly.
        assert_eq!(ease(easing, 1.5), 1.0, "{easing:?}");
        assert_eq!(ease(easing, -0.5), 0.0, "{easing:?}");
    }
}

#[test]
fn easing_midpoints() {
    assert_eq!(ease(Easing::Linear, 0.5), 0.5);
    assert_eq!(ease(Easing::EaseIn, 0.5), 0.125);
    assert_eq!(ease(Easing::EaseOut, 0.5), 0.875);
    assert_eq!(ease(Easing::EaseInOut, 0.5), 0.5);
    assert_eq!(ease(Easing::Smooth, 0.5), 0.5);
}

#[test]
fn easing_is_monotonic() {
    for easing in Easing::ALL {
        let mut last = 0.0;
        for i in 1..=100 {
            let value = ease(easing, f64::from(i) / 100.0);
            assert!(value >= last, "{easing:?} fell at {i}");
            last = value;
        }
    }
}

#[test]
fn progress_clocks() {
    let fade = spec(Kind::Fade);
    assert_eq!(progress(&fade, 0), 0.0);
    assert_eq!(progress(&fade, 250), 0.5);
    assert_eq!(progress(&fade, 500), 1.0);
    assert_eq!(progress(&fade, 10_000), 1.0);
    let instant = Spec {
        duration_ms: 0,
        ..fade
    };
    assert_eq!(progress(&instant, 0), 1.0);
    let eased = Spec {
        easing: Easing::EaseOut,
        ..fade
    };
    assert_eq!(progress(&eased, 250), 0.875);
}

#[test]
fn steps() {
    assert_eq!(total_steps(0), 1);
    assert_eq!(total_steps(1), 1);
    assert_eq!(total_steps(500), 32);
    assert_eq!(step_for_elapsed(500, 0), 0);
    assert_eq!(step_for_elapsed(500, 16), 1);
    assert_eq!(step_for_elapsed(500, 256), 16);
    assert_eq!(step_for_elapsed(500, 10_000), 32);
    assert_eq!(step_for_elapsed(0, 0), 0);
    assert_eq!(step_for_elapsed(0, 1_000), 1);
}

#[test]
fn skip_budget() {
    assert_eq!(skip_for(0), 0);
    assert_eq!(skip_for(FRAME_BUDGET_MS), 0);
    assert_eq!(skip_for(FRAME_BUDGET_MS + 1), 1);
    assert_eq!(skip_for(1_000), 1);
}

#[test]
fn instant_specs() {
    assert!(Spec::none().is_instant());
    assert!(
        Spec {
            kind: Kind::Fade,
            duration_ms: 0,
            ..Spec::none()
        }
        .is_instant()
    );
    assert!(!spec(Kind::Fade).is_instant());
}

#[test]
fn fade_endpoints() {
    let dims = (9, 7);
    for eased in [0.0, 1.0] {
        let sweep = Sweep::new(&spec(Kind::Fade), eased, dims);
        all_pixels(dims, |x, y| {
            assert_eq!(sweep.weight(x, y), eased, "({x},{y}) at {eased}");
        });
    }
}

#[test]
fn fade_mid_is_uniform() {
    let sweep = Sweep::new(&spec(Kind::Fade), 0.25, (9, 7));
    all_pixels((9, 7), |x, y| {
        assert_eq!(sweep.weight(x, y), 0.25, "({x},{y})");
    });
}

#[test]
fn wipe_endpoints() {
    for angle in [0.0, 45.0, 90.0, 135.0, 180.0, 270.0, 359.0] {
        let dims = (11, 9);
        let old = Sweep::new(
            &Spec {
                kind: Kind::Wipe,
                angle_deg: angle,
                ..spec(Kind::Wipe)
            },
            0.0,
            dims,
        );
        let new = Sweep::new(
            &Spec {
                kind: Kind::Wipe,
                angle_deg: angle,
                ..spec(Kind::Wipe)
            },
            1.0,
            dims,
        );
        all_pixels(dims, |x, y| {
            assert_eq!(old.weight(x, y), 0.0, "({x},{y}) at {angle}");
            assert_eq!(new.weight(x, y), 1.0, "({x},{y}) at {angle}");
        });
    }
}

#[test]
fn wipe_axis_directions() {
    let dims = (10, 6);
    let at = |angle: f64, eased: f64| {
        Sweep::new(
            &Spec {
                kind: Kind::Wipe,
                angle_deg: angle,
                ..spec(Kind::Wipe)
            },
            eased,
            dims,
        )
    };
    // 0 degrees: new from the left.
    let sweep = at(0.0, 0.5);
    assert_eq!(sweep.weight(0, 0), 1.0);
    assert_eq!(sweep.weight(9, 0), 0.0);
    // 180: new from the right.
    let sweep = at(180.0, 0.5);
    assert_eq!(sweep.weight(9, 3), 1.0);
    assert_eq!(sweep.weight(0, 3), 0.0);
    // 90: new from the top.
    let sweep = at(90.0, 0.5);
    assert_eq!(sweep.weight(5, 0), 1.0);
    assert_eq!(sweep.weight(5, 5), 0.0);
    // 270: new from the bottom.
    let sweep = at(270.0, 0.5);
    assert_eq!(sweep.weight(5, 5), 1.0);
    assert_eq!(sweep.weight(5, 0), 0.0);
    // A full turn is the same wipe.
    let (a, b) = (at(0.0, 0.3), at(360.0, 0.3));
    all_pixels(dims, |x, y| assert_eq!(a.weight(x, y), b.weight(x, y)));
}

#[test]
fn wipe_edge_position() {
    // 100 px wide, halfway: the edge sits at x = 50 (inset math:
    // edge = -1 + 0.5 * 102 = 50).
    let sweep = Sweep::new(&spec(Kind::Wipe), 0.5, (100, 10));
    assert_eq!(sweep.weight(50, 5), 1.0);
    assert_eq!(sweep.weight(51, 5), 0.0);
}

#[test]
fn grow_endpoints() {
    for pos in [(0.5, 0.5), (0.0, 0.0), (1.0, 1.0), (0.2, 0.8)] {
        let dims = (11, 9);
        let old = Sweep::new(
            &Spec {
                kind: Kind::Grow,
                pos,
                ..spec(Kind::Grow)
            },
            0.0,
            dims,
        );
        let new = Sweep::new(
            &Spec {
                kind: Kind::Grow,
                pos,
                ..spec(Kind::Grow)
            },
            1.0,
            dims,
        );
        all_pixels(dims, |x, y| {
            assert_eq!(old.weight(x, y), 0.0, "({x},{y}) at {pos:?}");
            assert_eq!(new.weight(x, y), 1.0, "({x},{y}) at {pos:?}");
        });
    }
}

#[test]
fn grow_from_center() {
    let dims = (101, 101);
    let sweep = Sweep::new(&spec(Kind::Grow), 0.5, dims);
    // The center is new long before the corners.
    assert_eq!(sweep.weight(50, 50), 1.0);
    assert_eq!(sweep.weight(0, 0), 0.0);
    assert_eq!(sweep.weight(100, 100), 0.0);
}

#[test]
fn grow_from_corner() {
    let dims = (101, 101);
    let sweep = Sweep::new(
        &Spec {
            kind: Kind::Grow,
            pos: (0.0, 0.0),
            ..spec(Kind::Grow)
        },
        0.25,
        dims,
    );
    assert_eq!(sweep.weight(0, 0), 1.0);
    assert_eq!(sweep.weight(100, 100), 0.0);
}

#[test]
fn blend_row_fade_midpoint() {
    let sweep = Sweep::new(&spec(Kind::Fade), 0.5, (2, 1));
    let old = [0u8, 0, 0, 0xff, 255, 255, 255, 0xff];
    let new = [255u8, 255, 255, 0xff, 0, 0, 0, 0xff];
    let mut out = [9u8; 8];
    blend_row(&sweep, 0, &old, &new, &mut out);
    // 127.5 rounds to 128 either way.
    assert_eq!(out, [128, 128, 128, 0xff, 128, 128, 128, 0xff]);
}

#[test]
fn blend_row_endpoints_exact() {
    let old = [10u8, 20, 30, 0x00, 200, 100, 50, 0x00];
    let new = [200u8, 100, 50, 0x00, 10, 20, 30, 0x00];
    for (eased, want) in [(0.0, old), (1.0, new)] {
        let sweep = Sweep::new(&spec(Kind::Fade), eased, (2, 1));
        let mut out = [9u8; 8];
        blend_row(&sweep, 0, &old, &new, &mut out);
        let mut expected = want;
        expected[3] = 0xff;
        expected[7] = 0xff;
        assert_eq!(out, expected, "at {eased}");
    }
}

#[test]
fn blend_row_wipe_matches_weights() {
    let dims = (7, 3);
    let sweep = Sweep::new(&spec(Kind::Wipe), 0.4, dims);
    let old = vec![0u8; 7 * 4];
    let new = vec![255u8; 7 * 4];
    for y in 0..3 {
        let mut out = vec![9u8; 7 * 4];
        blend_row(&sweep, y, &old, &new, &mut out);
        for x in 0..7 {
            let want = if sweep.weight(x, y) == 1.0 { 255 } else { 0 };
            assert_eq!(out[x as usize * 4], want, "({x},{y})");
            assert_eq!(out[x as usize * 4 + 3], 0xff, "({x},{y}) alpha");
        }
    }
}

#[test]
fn damage_fade_is_whole() {
    let dims = (80, 50);
    let damage = damage(Kind::Fade, 0.2, 0.5, dims, 0.0, (0.5, 0.5));
    assert_eq!(damage.n, 1);
    assert_eq!(damage.rects[0], Rect::whole(dims));
}

#[test]
fn damage_none_and_equal() {
    let dims = (80, 50);
    assert!(damage(Kind::None, 0.0, 1.0, dims, 0.0, (0.5, 0.5)).is_empty());
    assert!(damage(Kind::Fade, 0.3, 0.3, dims, 0.0, (0.5, 0.5)).is_empty());
    assert!(damage(Kind::Wipe, 0.7, 0.7, dims, 45.0, (0.5, 0.5)).is_empty());
}

#[test]
fn damage_wipe_axis_tight() {
    // 100x10, 0 degrees, 0.25 -> 0.5: edges at 24.5 and 50.
    let damage = damage(Kind::Wipe, 0.25, 0.5, (100, 10), 0.0, (0.5, 0.5));
    assert_eq!(damage.n, 1);
    assert_eq!(
        damage.rects[0],
        Rect {
            x: 24,
            y: 0,
            w: 26,
            h: 10,
        }
    );
}

#[test]
fn damage_grow_bbox() {
    let dims = (100, 100);
    let damage = damage(Kind::Grow, 0.0, 0.5, dims, 0.0, (0.5, 0.5));
    assert_eq!(damage.n, 1);
    let rect = damage.rects[0];
    // Center (50, 50), radius 0.5 * (hypot(50, 50) + 2) - 1.
    let r = 0.5 * (50.0_f64.hypot(50.0) + 2.0) - 1.0;
    assert_eq!(rect.x, (50.0 - r).floor() as u32);
    assert_eq!(rect.w, (50.0 + r).ceil() as u32 - rect.x);
    assert_eq!(rect.y, rect.x);
    assert_eq!(rect.h, rect.w);
}

#[test]
fn damage_covers_every_changed_pixel() {
    // The load-bearing property: brute force over small buffers, several
    // angles and centers, both directions. Every pixel whose weight moved
    // must sit inside a damage rect.
    let dims = (37, 23);
    let cases = [
        (Kind::Fade, 0.0, (0.5, 0.5)),
        (Kind::Wipe, 0.0, (0.5, 0.5)),
        (Kind::Wipe, 90.0, (0.5, 0.5)),
        (Kind::Wipe, 180.0, (0.5, 0.5)),
        (Kind::Wipe, 270.0, (0.5, 0.5)),
        (Kind::Wipe, 30.0, (0.5, 0.5)),
        (Kind::Wipe, 123.0, (0.5, 0.5)),
        (Kind::Grow, 0.0, (0.5, 0.5)),
        (Kind::Grow, 0.0, (0.0, 0.0)),
        (Kind::Grow, 0.0, (1.0, 0.25)),
    ];
    for (kind, angle, pos) in cases {
        let base = Spec {
            kind,
            angle_deg: angle,
            pos,
            ..spec(kind)
        };
        for (prev, now) in [(0.0, 0.4), (0.4, 0.75), (0.75, 1.0), (0.9, 0.1)] {
            let before = Sweep::new(&base, prev, dims);
            let after = Sweep::new(&base, now, dims);
            let damage = damage(kind, prev, now, dims, angle, pos);
            let covers = |x: u32, y: u32| {
                damage
                    .iter()
                    .any(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            };
            all_pixels(dims, |x, y| {
                if before.weight(x, y) != after.weight(x, y) {
                    assert!(
                        covers(x, y),
                        "{kind:?} {angle} {pos:?} {prev}->{now}: ({x},{y}) changed but not damaged"
                    );
                }
            });
        }
    }
}

#[test]
fn damage_never_exceeds_buffer() {
    let dims = (37, 23);
    for (kind, angle, pos) in [
        (Kind::Fade, 0.0, (0.5, 0.5)),
        (Kind::Wipe, 33.0, (0.5, 0.5)),
        (Kind::Grow, 0.0, (1.0, 1.0)),
    ] {
        for (prev, now) in [(0.0, 1.0), (0.0, 0.01), (0.99, 1.0)] {
            for rect in damage(kind, prev, now, dims, angle, pos).iter() {
                assert!(rect.x + rect.w <= dims.0, "{rect:?}");
                assert!(rect.y + rect.h <= dims.1, "{rect:?}");
            }
        }
    }
}

#[test]
fn frame_bytes_bound() {
    assert_eq!(frame_bytes((3840, 2160)), Some(33_177_600));
    assert_eq!(frame_bytes((0, 0)), Some(0));
    assert_eq!(frame_bytes((u32::MAX, u32::MAX)), None);
}

#[test]
fn hostile_geometry_cannot_panic() {
    // Zero-size and degenerate inputs must return, not panic or NaN.
    let tiny = Spec {
        kind: Kind::Wipe,
        angle_deg: 0.0,
        ..spec(Kind::Wipe)
    };
    assert!(damage(Kind::Wipe, 0.0, 0.5, (0, 0), 0.0, (0.5, 0.5)).is_empty());
    let sweep = Sweep::new(&tiny, 0.5, (1, 1));
    assert!(sweep.weight(0, 0).is_finite());
    let damage = damage(Kind::Grow, 0.0, 0.5, (1, 1), 0.0, (0.5, 0.5));
    for rect in damage.iter() {
        assert!(rect.x + rect.w <= 1 && rect.y + rect.h <= 1);
    }
    // Untouched `None` sweeps read as new (the driver never builds one).
    assert_eq!(Sweep::new(&Spec::none(), 0.3, (4, 4)).weight(1, 1), 1.0);
}

#[test]
fn assemble_rules() {
    let spell = str::to_owned;
    // Nothing: none.
    assert_eq!(
        assemble(None, None, None, None, None, spell),
        Ok(Spec::none())
    );
    // An explicit `none` ignores the rest.
    assert_eq!(
        assemble(Some(Kind::None), Some("5"), None, None, None, spell),
        Ok(Spec::none())
    );
    // Parameters without a kind are refused, naming the parameter.
    assert_eq!(
        assemble(None, Some("5"), None, None, None, spell),
        Err(ParseError::Orphan("duration-ms".to_owned()))
    );
    assert_eq!(
        assemble(None, None, None, None, Some("0,0"), spell),
        Err(ParseError::Orphan("position".to_owned()))
    );
    // A kind fills the defaults.
    assert_eq!(
        assemble(Some(Kind::Fade), None, None, None, None, spell),
        Ok(Spec {
            kind: Kind::Fade,
            duration_ms: DEFAULT_DURATION_MS,
            easing: Easing::EaseOut,
            angle_deg: 0.0,
            pos: (0.5, 0.5),
        })
    );
    // Failures report the value.
    assert_eq!(
        assemble(Some(Kind::Fade), Some("-1"), None, None, None, spell),
        Err(ParseError::BadDuration("-1".to_owned()))
    );
}
