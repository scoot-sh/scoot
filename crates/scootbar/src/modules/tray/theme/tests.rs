//! Fixture theme tests: a `hicolor` tree in a tmp dir, plus the
//! traversal and bomb cases. No machine theme is touched: every test
//! passes explicit bases.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{is_valid_name, is_valid_theme_path, load};

/// A scratch dir under the system tmp area, unique per test.
fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "scootbar-tray-theme-{}-{}-{}",
        name,
        std::process::id(),
        scratch_counter()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn scratch_counter() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Removes the scratch dir and everything under it.
fn cleanup(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

/// Encodes a solid `width × height` RGBA PNG.
fn encode_png(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let data = pixel.repeat(width as usize * height as usize);
        writer.write_image_data(&data).unwrap();
    }
    bytes
}

/// Writes `bytes` to `root/hicolor/22x22/apps/<name>.png`, parents made.
fn write_hicolor(root: &Path, name: &str) -> PathBuf {
    write_hicolor_size(root, "22x22/apps", name)
}

fn write_hicolor_size(root: &Path, size: &str, name: &str) -> PathBuf {
    let dir = root.join("hicolor").join(size);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    std::fs::write(&path, encode_png(4, 4, [200, 30, 30, 255])).unwrap();
    path
}

/// Writes `width × height` `pixel` PNG bytes to
/// `root/hicolor/<size>/<name>.png`, parents made.
fn write_png_size(root: &Path, size: &str, name: &str, width: u32, height: u32, pixel: [u8; 4]) {
    let dir = root.join("hicolor").join(size);
    std::fs::create_dir_all(&dir).unwrap();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&pixel.repeat(width as usize * height as usize))
            .unwrap();
    }
    std::fs::write(dir.join(format!("{name}.png")), &bytes).unwrap();
}

#[test]
fn hostile_names_are_refused_without_touching_the_disk() {
    assert!(!is_valid_name(""));
    assert!(!is_valid_name("../escape"));
    assert!(!is_valid_name("a/b"));
    assert!(!is_valid_name("/abs/path"));
    assert!(!is_valid_name(".hidden"));
    assert!(!is_valid_name(".."));
    assert!(!is_valid_name("back\\slash"));
    assert!(!is_valid_name(&"x".repeat(129)));
    assert!(is_valid_name("pasystray"));
    assert!(is_valid_name("copyq"));
    assert!(is_valid_name("keepassxc"));
    // A traversal name resolves to nothing even against a real base.
    let root = scratch("names");
    let bases = [root.clone()];
    assert!(load("id", "../escape", None, &bases).is_none());
    assert!(load("id", "/etc/passwd", None, &bases).is_none());
    assert!(load("id", "", None, &bases).is_none());
    cleanup(&root);
}

#[test]
fn hostile_theme_paths_are_ignored() {
    assert!(!is_valid_theme_path(""));
    assert!(!is_valid_theme_path("relative/path"));
    assert!(!is_valid_theme_path("/has/../traversal"));
    assert!(!is_valid_theme_path("/has\0nul"));
    assert!(is_valid_theme_path("/usr/share/icons/custom"));
    // A relative theme path never becomes a search root: the hicolor
    // icon is still found through the bases.
    let root = scratch("themepath");
    write_hicolor(&root, "app");
    let bases = [root.clone()];
    assert!(load("id", "app", Some("relative/path"), &bases).is_some());
    assert!(load("id", "app", Some("/no/such/dir"), &bases).is_some());
    cleanup(&root);
}

#[test]
fn a_themed_name_resolves_through_hicolor() {
    let root = scratch("hicolor");
    write_hicolor(&root, "pasystray");
    let bases = [root.clone()];
    let icon = load("item-id", "pasystray", None, &bases).expect("the fixture icon");
    assert_eq!(icon.side(), 4);
    // The id is stable for the same item and file: a steady icon hits
    // the shared cache every frame after the first miss.
    let again = load("item-id", "pasystray", None, &bases).expect("the fixture icon");
    assert_eq!(icon.id(), again.id());
    cleanup(&root);
}

#[test]
fn icon_theme_path_is_searched_first() {
    let base = scratch("base");
    let custom = scratch("custom");
    write_hicolor_size(&base, "22x22/apps", "app");
    // Same name, different color through the item's own theme path.
    let dir = custom.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.png");
    std::fs::write(&path, encode_png(4, 4, [10, 200, 10, 255])).unwrap();
    let bases = [base.clone()];
    let icon =
        load("item-id", "app", Some(custom.to_str().unwrap()), &bases).expect("the custom icon");
    // 10 green: the theme path won over hicolor's red.
    let other = load("item-id", "app", None, &bases).expect("the hicolor icon");
    assert_ne!(icon.id(), other.id());
    cleanup(&base);
    cleanup(&custom);
}

#[test]
fn a_missing_or_unthemed_name_stays_hidden() {
    let root = scratch("missing");
    let bases = [root.clone()];
    // Nothing installed: no icon, no panic.
    assert!(load("item-id", "no-such-icon", None, &bases).is_none());
    // An SVG beside the name is not decoded (no renderer is linked):
    // without a PNG the name stays hidden, as before.
    std::fs::create_dir_all(root.join("hicolor/22x22/apps")).unwrap();
    std::fs::write(
        root.join("hicolor/22x22/apps/svgonly.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
    )
    .unwrap();
    assert!(load("item-id", "svgonly", None, &bases).is_none());
    cleanup(&root);
}

#[test]
fn a_symlink_escape_is_refused() {
    let root = scratch("symlink");
    let outside = scratch("outside");
    std::fs::write(outside.join("real.png"), encode_png(4, 4, [1, 2, 3, 255])).unwrap();
    let dir = root.join("hicolor/22x22/apps");
    std::fs::create_dir_all(&dir).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.join("real.png"), dir.join("linked.png")).unwrap();
    let bases = [root.clone()];
    #[cfg(unix)]
    assert!(load("item-id", "linked", None, &bases).is_none());
    cleanup(&root);
    cleanup(&outside);
}

#[test]
fn a_bomb_declaring_a_huge_image_is_refused_without_a_big_alloc() {
    // A valid PNG with its IHDR dimensions patched huge: the header
    // check refuses it after a few dozen bytes, before any pixel buffer.
    let mut bomb = encode_png(4, 4, [1, 2, 3, 255]);
    let width: u32 = 100_000;
    let height: u32 = 100_000;
    bomb[16..20].copy_from_slice(&width.to_be_bytes());
    bomb[20..24].copy_from_slice(&height.to_be_bytes());
    assert!(super::decode("item-id", &bomb).is_none());
    // Truncated and garbage inputs are refused the same way.
    assert!(super::decode("item-id", &bomb[..40]).is_none());
    assert!(super::decode("item-id", b"not a png at all").is_none());
    assert!(super::decode("item-id", &[]).is_none());
}

#[test]
fn an_oversized_file_is_skipped() {
    let root = scratch("oversize");
    let dir = root.join("hicolor/22x22/apps");
    std::fs::create_dir_all(&dir).unwrap();
    // Past the file bound but a valid name: skipped, never read whole.
    let big = vec![0u8; super::MAX_FILE as usize + 1];
    std::fs::write(dir.join("big.png"), &big).unwrap();
    let bases = [root.clone()];
    assert!(load("item-id", "big", None, &bases).is_none());
    cleanup(&root);
}

#[test]
fn a_fifo_is_refused_without_blocking() {
    // The post-open type check, directly: a FIFO where the icon file
    // would be (as after a swap between the metadata check and the
    // open). The read runs on a worker behind a 10 s watchdog: without
    // the `O_NONBLOCK` open plus the regular-file `fstat`, the open
    // blocks and the watchdog fires.
    let root = scratch("fifo");
    let fifo = root.join("fifo.png");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::from_bits_truncate(0o600),
    )
    .unwrap();
    let path = fifo.clone();
    let (done, waited) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(super::read_file("item-id", &path));
    });
    match waited.recv_timeout(Duration::from_secs(10)) {
        Ok(None) => {}
        Ok(Some(_)) => panic!("a FIFO decoded into an icon"),
        Err(_) => panic!("blocked opening a FIFO for 10 s"),
    }
    cleanup(&root);
}

#[test]
fn a_large_icon_is_stored_at_the_bound() {
    // A 128 px icon still draws (real themes ship 128 and 256 px sizes
    // for launchers), but the stored entry is the pixmap path's bound,
    // not the decoded size.
    let bytes = encode_png(128, 128, [10, 200, 10, 255]);
    let icon = super::decode("item-id", &bytes).expect("a large icon still draws");
    assert!(
        icon.side() <= super::super::MAX_STORED_SIDE,
        "stored side {} past the bound",
        icon.side()
    );
}

#[test]
fn thirty_two_large_icons_fit_the_total_budget() {
    // One 128 px icon per item, the whole tray of them: 32 x 64 KiB
    // without the stored cap is 2 MiB over the 512 KiB total budget.
    let root = scratch("total");
    let bases = [root.clone()];
    let mut total = 0usize;
    for n in 0..super::super::MAX_ITEMS {
        let name = format!("icon{n}");
        write_png_size(
            &root,
            "128x128/apps",
            &name,
            128,
            128,
            [n as u8, 200, 30, 255],
        );
        let icon = super::load("item", &name, None, &bases).expect("the fixture icon");
        total += icon.side() as usize * icon.side() as usize * 4;
    }
    assert!(
        total <= super::MAX_THEME_TOTAL_BYTES,
        "{total} stored bytes over the {} budget",
        super::MAX_THEME_TOTAL_BYTES
    );
    cleanup(&root);
}

#[test]
fn the_closest_size_wins() {
    // The same name in 22x22 (red) and 48x48 (green): drawn at 24 the
    // 22 px entry wins, drawn at 48 the 48 px one does (the freedesktop
    // closest-size rule among the sizes present, not first-match).
    let root = scratch("closest");
    write_png_size(&root, "22x22/apps", "both", 22, 22, [200, 30, 30, 255]);
    write_png_size(&root, "48x48/apps", "both", 48, 48, [30, 200, 30, 255]);
    write_png_size(&root, "22x22/apps", "red", 22, 22, [200, 30, 30, 255]);
    write_png_size(&root, "48x48/apps", "green", 48, 48, [30, 200, 30, 255]);
    let bases = [root.clone()];
    let red = super::load_for_side("id", "red", None, &bases, 24).expect("the red icon");
    let green = super::load_for_side("id", "green", None, &bases, 48).expect("the green icon");
    let near = super::load_for_side("id", "both", None, &bases, 24).expect("drawn at 24");
    assert_eq!(near.id(), red.id(), "drawn 24 resolves past the 22x22 entry");
    let far = super::load_for_side("id", "both", None, &bases, 48).expect("drawn at 48");
    assert_eq!(far.id(), green.id(), "drawn 48 resolves past the 48x48 entry");
    cleanup(&root);
}

#[test]
fn a_name_with_an_extension_resolves_like_the_bare_name() {
    // An `IconName` is a name without extension, but many apps send one
    // with: one trailing `.png` is accepted. `.svg`/`.xpm` name no PNG
    // the bar decodes, so they stay hidden like any unresolvable name.
    let root = scratch("ext");
    write_hicolor(&root, "app");
    let bases = [root.clone()];
    let bare = load("id", "app", None, &bases).expect("the bare name");
    let suffixed = load("id", "app.png", None, &bases).expect("the name with .png");
    assert_eq!(bare.id(), suffixed.id());
    std::fs::write(
        root.join("hicolor/22x22/apps/vec.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
    )
    .unwrap();
    assert!(load("id", "vec.svg", None, &bases).is_none());
    std::fs::write(root.join("hicolor/22x22/apps/legacy.xpm"), "/* XPM */").unwrap();
    assert!(load("id", "legacy.xpm", None, &bases).is_none());
    cleanup(&root);
}
