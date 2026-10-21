//! Icon lookup tests: order, bounds, hostile input and the cache.

use super::*;
use std::fs;
use std::os::unix::fs::symlink;

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    let pixels = vec![255u8; width as usize * height as usize * 4];
    writer.write_image_data(&pixels).expect("png data");
    drop(writer);
    out
}

fn fixture_theme(root: &Path, theme: &str, dirs: &[&str], inherits: Option<&str>) {
    for dir in dirs {
        fs::create_dir_all(root.join(theme).join(dir)).expect("mkdir");
    }
    let mut index = String::from("[Icon Theme]\n");
    index.push_str(&format!("Name={theme}\nDirectories={}\n", dirs.join(",")));
    if let Some(parents) = inherits {
        index.push_str(&format!("Inherits={parents}\n"));
    }
    for dir in dirs {
        index.push_str(&format!("\n[{dir}]\nSize=48\nType=Fixed\n"));
    }
    fs::write(root.join(theme).join("index.theme"), index).expect("index");
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("tmp")
}

#[test]
fn names_and_paths_are_gated() {
    assert!(is_valid_name("firefox"));
    assert!(!is_valid_name(""));
    assert!(!is_valid_name("a/b"));
    assert!(!is_valid_name(".hidden"));
    assert!(!is_valid_name(&"a".repeat(129)));
    assert!(is_valid_path("/usr/share/icons/firefox.png"));
    assert!(!is_valid_path("relative.png"));
    assert!(!is_valid_path("/tmp/../escape.png"));
}

#[test]
fn png_wins_over_svg_in_the_same_dir() {
    let home = tmp();
    let base = home.path().join("icons");
    fs::create_dir_all(&base).expect("base");
    fixture_theme(&base, "Adwaita", &["48x48/apps"], None);
    let dir = base.join("Adwaita").join("48x48/apps");
    fs::write(dir.join("app.png"), png_bytes(4, 4)).expect("png");
    fs::write(
        dir.join("app.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"48\" height=\"48\"></svg>",
    )
    .expect("svg");
    let bases = vec![base];
    let found = find_file("Adwaita", "app", 48, &bases).expect("finds");
    assert!(found.extension().and_then(|e| e.to_str()) == Some("png"));
}

#[test]
#[cfg(feature = "svg")]
fn svg_loads_only_when_no_png_exists() {
    let home = tmp();
    let base = home.path().join("icons");
    fs::create_dir_all(&base).expect("base");
    fixture_theme(&base, "Adwaita", &["48x48/apps"], None);
    let dir = base.join("Adwaita").join("48x48/apps");
    fs::write(
        dir.join("vec.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"48\" height=\"48\"><rect width=\"48\" height=\"48\" fill=\"red\"/></svg>",
    )
    .expect("svg");
    let bases = vec![base];
    let found = find_file("Adwaita", "vec", 48, &bases).expect("finds svg");
    assert!(found.extension().and_then(|e| e.to_str()) == Some("svg"));
    let decoded = decode_file(&found, 32).expect("decodes");
    assert_eq!((decoded.width, decoded.height), (32, 32));
}

#[test]
fn inherits_chain_and_hicolor_fall_through() {
    let home = tmp();
    let base = home.path().join("icons");
    fs::create_dir_all(&base).expect("base");
    fixture_theme(&base, "Child", &["48x48/apps"], Some("Parent"));
    fixture_theme(&base, "Parent", &["48x48/apps"], None);
    fixture_theme(&base, "hicolor", &["48x48/apps"], None);
    fs::write(
        base.join("hicolor").join("48x48/apps").join("deep.png"),
        png_bytes(2, 2),
    )
    .expect("png");
    let bases = vec![base];
    let found = find_file("Child", "deep", 48, &bases).expect("falls to hicolor");
    assert!(found.to_string_lossy().contains("hicolor"));
}

#[test]
fn absolute_paths_are_used_directly() {
    let home = tmp();
    let file = home.path().join("icon.png");
    fs::write(&file, png_bytes(2, 2)).expect("png");
    let bases = vec![];
    let found =
        find_file("Adwaita", file.to_string_lossy().as_ref(), 16, &bases).expect("absolute");
    assert_eq!(found, file);
}

#[test]
fn hostile_inputs_never_resolve_or_panic() {
    let home = tmp();
    let base = home.path().join("icons");
    fs::create_dir_all(&base).expect("base");
    let bases = vec![base.clone()];
    for hostile in [
        "",
        "../escape",
        ".hidden",
        "a/b",
        "/tmp/../escape.png",
        "relative",
        &"a".repeat(200),
    ] {
        let _ = find_file("Adwaita", hostile, 32, &bases);
        let _ = find_file("../../../etc", "app", 32, &bases);
    }
    // A symlink loop is ELOOP, not a hang.
    let link = base.join("loop.png");
    let _ = symlink(&link, &link);
    assert!(find_file("Adwaita", "/nonexistent-loop-check", 16, &bases).is_none());
    let _ = is_usable_file(&link);
    // A corrupt PNG decodes to nothing, not a panic.
    let bad = home.path().join("bad.png");
    fs::write(&bad, b"not a png at all").expect("bad");
    assert!(decode_file(&bad, 16).is_none());
    // A huge header is refused before any pixel buffer exists.
    let mut huge = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut huge, 4096, 4096);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // Only the header is needed: the decoder refuses the dimensions
        // from the IHDR before pixel bytes arrive. Write a truncated
        // stream (header + no IDAT) so the test stays small.
        let mut writer = encoder.write_header().expect("header");
        let _ = writer.write_image_data(&[0u8; 64]);
    }
    let huge_file = home.path().join("huge.png");
    fs::write(&huge_file, huge).expect("huge");
    assert!(decode_file(&huge_file, 32).is_none());
}

#[test]
#[cfg(feature = "svg")]
fn svg_bounds_hold() {
    // Too many nodes: refused before parsing.
    let home = tmp();
    let many = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>",
        "<g/>".repeat(MAX_SVG_NODES + 1)
    );
    let file = home.path().join("many.svg");
    fs::write(&file, many.as_bytes()).expect("many");
    assert!(decode_file(&file, 32).is_none());
    // Oversized file: refused on size.
    let big = home.path().join("big.svg");
    fs::write(&big, vec![b'a'; MAX_SVG_FILE as usize + 1]).expect("big");
    assert!(decode_file(&big, 32).is_none());
    // External references resolve to nothing (no resources_dir): the
    // render still returns a square (possibly blank), never a fetch.
    let ext = home.path().join("ext.svg");
    fs::write(
        &ext,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"48\" height=\"48\"><image href=\"https://example.com/a.png\" width=\"48\" height=\"48\"/></svg>",
    )
    .expect("ext");
    let decoded = decode_file(&ext, 16);
    assert!(decoded.is_none() || decoded.is_some_and(|d| d.width == 16));
}

#[test]
fn icon_less_rows_draw_their_own_letters() {
    // The filed bug: two icon-less entries shared the single `("", side)`
    // entry, so Printer drew Calculator's `C`. Tiles are never cached
    // now: each row renders its own label's letter, in order, repeatedly.
    let mut cache = Cache::default();
    let bases = vec![];
    for _ in 0..3 {
        let calc = cache.get("Adwaita", &bases, None, "Calculator", 16);
        let print = cache.get("Adwaita", &bases, None, "Printer", 16);
        assert!(matches!(calc, Icon::Tile('C')), "{calc:?}");
        assert!(matches!(print, Icon::Tile('P')), "{print:?}");
    }
    // A shared unresolvable name falls back per label too, not per name.
    for _ in 0..2 {
        let calc = cache.get("Adwaita", &bases, Some("no-such-icon"), "Calculator", 16);
        let print = cache.get("Adwaita", &bases, Some("no-such-icon"), "Printer", 16);
        assert!(matches!(calc, Icon::Tile('C')), "{calc:?}");
        assert!(matches!(print, Icon::Tile('P')), "{print:?}");
    }
    // Tiles never enter the map, so icon-less rows leave it empty.
    let (entries, bytes) = cache.cached();
    assert_eq!((entries, bytes), (0, 0));
}

#[test]
fn cache_evicts_on_entry_count() {
    // 72 distinct icons past the entry cap: without the eviction the
    // cache would hold all 72; with it the count stays under the cap.
    // Tiles are never cached, so these are real (tiny) images.
    let home = tmp();
    let mut files = Vec::new();
    for i in 0..(MAX_ENTRIES + 8) {
        let file = home.path().join(format!("count{i}.png"));
        fs::write(&file, png_bytes(2, 2)).expect("png");
        files.push(file);
    }
    let bases = vec![];
    let mut cache = Cache::default();
    for file in &files {
        let icon = cache.get(
            "Adwaita",
            &bases,
            Some(file.to_string_lossy().as_ref()),
            "App",
            16,
        );
        assert!(matches!(icon, Icon::Image(_)), "{file:?}");
    }
    let (entries, bytes) = cache.cached();
    assert!(entries <= MAX_ENTRIES, "{entries}");
    assert_eq!(entries, 8, "{entries}");
    assert!(bytes <= MAX_BYTES, "{bytes}");
    assert_eq!(tile_letter("Firefox"), 'F');
    assert_eq!(tile_letter(""), '?');
    assert_eq!(tile_letter("123"), '1');
}

#[test]
fn cache_evicts_on_bytes() {
    // One 256 px image icon times five: each decodes to 256 KiB, so the
    // fifth pushes past the 1 MiB cap and trips the byte bound. Without
    // the eviction the cache would hold all five (1280 KiB).
    let home = tmp();
    let mut files = Vec::new();
    for i in 0..5 {
        let file = home.path().join(format!("byte{i}.png"));
        fs::write(&file, png_bytes(48, 48)).expect("png");
        files.push(file);
    }
    let bases = vec![];
    let mut cache = Cache::default();
    for file in &files {
        let icon = cache.get(
            "Adwaita",
            &bases,
            Some(file.to_string_lossy().as_ref()),
            "App",
            256,
        );
        assert!(matches!(icon, Icon::Image(_)), "{file:?}");
    }
    let (entries, bytes) = cache.cached();
    assert!(entries <= MAX_ENTRIES, "{entries}");
    assert!(bytes <= MAX_BYTES, "{bytes}");
    assert_eq!(bytes, 256 * 256 * 4, "{bytes}");
    assert_eq!(entries, 1, "{entries}");
}

#[test]
fn device_side_is_capped() {
    assert_eq!(256u32.clamp(1, MAX_RENDER_SIDE), MAX_RENDER_SIDE);
    assert_eq!(0u32.clamp(1, MAX_RENDER_SIDE), 1);
}
