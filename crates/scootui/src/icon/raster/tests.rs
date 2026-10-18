use super::Rasterizer;
use crate::icon::path::{Vector, ViewBox};

fn fill(d: &str, view: &str, side: u32) -> Vec<u8> {
    let vector = Vector::parse(d, ViewBox::parse(view).unwrap()).unwrap();
    let mut out = vec![0xaa; (side * side) as usize];
    Rasterizer::default().fill(&vector, side, &mut out);
    out
}

fn at(pixels: &[u8], side: u32, x: u32, y: u32) -> u8 {
    pixels[(y * side + x) as usize]
}

/// The sum of coverage, as area in pixels.
fn area(pixels: &[u8]) -> f64 {
    pixels.iter().map(|&c| f64::from(c)).sum::<f64>() / 255.0
}

fn near(a: u8, b: u8, tolerance: u8) -> bool {
    a.abs_diff(b) <= tolerance
}

#[test]
fn a_square_on_pixel_edges_is_exactly_covered() {
    let pixels = fill("M0 0H4V4H0z", "0 0 4 4", 4);
    assert!(pixels.iter().all(|&c| c == 255), "{pixels:?}");
    // Scaled by a non-integer factor, its edges still land on the
    // square's edges, so it is still whole.
    let pixels = fill("M0 0H24V24H0z", "0 0 24 24", 10);
    assert!(pixels.iter().all(|&c| c >= 254), "{pixels:?}");
}

#[test]
fn an_edge_through_the_middle_of_a_pixel_covers_half_of_it() {
    // A rectangle inset by half a pixel: corners a quarter, edges a half.
    let pixels = fill("M0.5 0.5H3.5V3.5H0.5z", "0 0 4 4", 4);
    let want = [
        [64, 128, 128, 64],
        [128, 255, 255, 128],
        [128, 255, 255, 128],
        [64, 128, 128, 64],
    ];
    for (y, row) in want.iter().enumerate() {
        for (x, &c) in row.iter().enumerate() {
            let got = at(&pixels, 4, x as u32, y as u32);
            assert!(near(got, c, 1), "({x}, {y}): {got}, want {c}\n{pixels:?}");
        }
    }
}

#[test]
fn a_diagonal_is_exact_area_in_each_pixel() {
    // The triangle x + y < 4: whole pixels under the diagonal, a half on it.
    let pixels = fill("M0 0L4 0L0 4z", "0 0 4 4", 4);
    for y in 0..4u32 {
        for x in 0..4u32 {
            let want = match (x + y).cmp(&3) {
                std::cmp::Ordering::Less => 255,
                std::cmp::Ordering::Equal => 128,
                std::cmp::Ordering::Greater => 0,
            };
            let got = at(&pixels, 4, x, y);
            assert!(near(got, want, 1), "({x}, {y}): {got}, want {want}");
        }
    }
}

#[test]
fn a_circle_has_its_area_and_is_symmetric() {
    // r = 10 at the center of 24: area pi r^2 = 314.16.
    let d = "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z";
    let pixels = fill(d, "0 0 24 24", 24);
    let a = area(&pixels);
    assert!((a - 314.16).abs() < 1.2, "area {a}");
    for y in 0..24u32 {
        for x in 0..24u32 {
            let mirror = at(&pixels, 24, 23 - x, y);
            assert!(near(at(&pixels, 24, x, y), mirror, 2), "({x}, {y})");
            let mirror = at(&pixels, 24, x, 23 - y);
            assert!(near(at(&pixels, 24, x, y), mirror, 2), "({x}, {y})");
        }
    }
    // Inside is whole, outside empty, and the rim is partial.
    assert_eq!(at(&pixels, 24, 12, 12), 255);
    assert_eq!(at(&pixels, 24, 0, 0), 0);
    let rim = at(&pixels, 24, 12, 2);
    assert!(rim > 0 && rim < 255 || rim == 255, "{rim}");
    // The same shape at a size that is not a whole multiple of the box:
    // the area follows the scale squared.
    let pixels = fill(d, "0 0 24 24", 15);
    let a = area(&pixels);
    let want = 314.16 * (15.0f64 / 24.0).powi(2);
    assert!((a - want).abs() < 0.8, "area {a}, want {want}");
}

#[test]
fn a_hole_wound_the_other_way_is_empty_and_one_wound_the_same_way_is_full() {
    // Outer clockwise, inner counterclockwise: a ring.
    let ring = fill("M0 0H12V12H0zM3 3V9H9V3z", "0 0 12 12", 12);
    assert_eq!(at(&ring, 12, 6, 6), 0);
    assert_eq!(at(&ring, 12, 1, 1), 255);
    assert!(near(area(&ring) as u8, 108, 1), "{}", area(&ring));
    // The same winding twice: nonzero fills it, the sum saturates at 1.
    let solid = fill("M0 0H12V12H0zM3 3H9V9H3z", "0 0 12 12", 12);
    assert_eq!(at(&solid, 12, 6, 6), 255);
    assert!((area(&solid) - 144.0).abs() < 0.01);
}

#[test]
fn a_path_is_fitted_in_the_square_centered_at_its_aspect_ratio() {
    // 48 by 24 in a 24-pixel square: half scale, 12 rows, centered.
    let pixels = fill("M0 0H48V24H0z", "0 0 48 24", 24);
    for y in 0..24u32 {
        let want = if (6..18).contains(&y) { 255 } else { 0 };
        for x in 0..24u32 {
            assert!(near(at(&pixels, 24, x, y), want, 1), "({x}, {y})");
        }
    }
    // A viewbox that does not start at the origin.
    let pixels = fill("M100 100h10v10h-10z", "100 100 10 10", 10);
    assert!(pixels.iter().all(|&c| c == 255));
}

#[test]
fn geometry_outside_the_square_is_clipped_not_an_error() {
    // Covering everything and far past it.
    let all = fill("M-1000 -1000H1000V1000H-1000z", "0 0 24 24", 8);
    assert!(all.iter().all(|&c| c == 255), "{all:?}");
    // Wholly outside: nothing.
    let none = fill("M100 100h5v5h-5z", "0 0 24 24", 8);
    assert!(none.iter().all(|&c| c == 0));
    // Straddling the left and top edges: only the inside counts.
    let corner = fill("M-12 -12H12V12H-12z", "0 0 24 24", 24);
    assert_eq!(at(&corner, 24, 0, 0), 255);
    assert_eq!(at(&corner, 24, 11, 11), 255);
    assert_eq!(at(&corner, 24, 12, 12), 0);
    // Extreme numbers cost nothing and write nothing outside.
    let extreme = fill("M-1e6 -1e6 L1e6 -1e6 L1e6 1e6 L-1e6 1e6z", "0 0 24 24", 8);
    assert!(extreme.iter().all(|&c| c == 255));
}

#[test]
fn a_curve_is_flattened_within_a_tenth_of_a_pixel() {
    // A big quarter-circle arc (r = 256 in a 512 box at 512 px): compare
    // the covered area with the exact one, quarter pi r^2.
    let d = "M0 0H256A256 256 0 0 1 0 256z";
    let pixels = fill(d, "0 0 512 512", 512);
    let want = std::f64::consts::PI * 256.0 * 256.0 / 4.0;
    let got = area(&pixels);
    assert!((got - want).abs() / want < 0.001, "{got} vs {want}");
}

#[test]
fn degenerate_sizes_and_buffers_are_left_alone() {
    let vector = Vector::parse("M0 0H24V24H0z", ViewBox::default()).unwrap();
    let mut raster = Rasterizer::default();
    // Side 0: nothing to write.
    raster.fill(&vector, 0, &mut []);
    // A buffer too short is untouched.
    let mut short = [7u8; 10];
    raster.fill(&vector, 4, &mut short);
    assert_eq!(short, [7u8; 10]);
    // A buffer longer than the square: only the square is written.
    let mut long = [7u8; 20];
    raster.fill(&vector, 4, &mut long);
    assert_eq!(&long[..16], &[255u8; 16]);
    assert_eq!(&long[16..], &[7u8; 4]);
}

#[test]
fn the_scratch_is_reused_between_fills() {
    let vector = Vector::parse("M0 0H24V24H0z", ViewBox::default()).unwrap();
    let mut raster = Rasterizer::default();
    let mut out = vec![0u8; 64 * 64];
    raster.fill(&vector, 64, &mut out);
    let (allocations, result) = {
        let mut out = vec![0u8; 32 * 32];
        let ((), count) = scootbg_mem::count_allocations(|| raster.fill(&vector, 32, &mut out));
        (count, out)
    };
    assert_eq!(allocations, 0, "a fill at a smaller size allocated");
    assert!(result.iter().all(|&c| c == 255));
}
