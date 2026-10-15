//! `check-image` tests: a valid file, a corrupt one, a header claiming
//! more pixels than the cap (refused before any allocation the size of the
//! image), a FIFO (refused without hanging), a symlink, a directory, and a
//! missing file.

use std::path::PathBuf;

use super::check;

/// A scratch file with `bytes` in it.
fn scratch(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scootbg-check-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    let path = dir.join("image.png");
    std::fs::write(&path, bytes).expect("a scratch image");
    path
}

/// A valid 1x1 RGB PNG (69 bytes).
const VALID: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xc9, 0xfe, 0x92, 0xef, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

/// The same file with its IHDR claiming 100000x100000 (10 gigapixels):
/// the pixel cap refuses it from the header, before anything is
/// allocated for its pixels.
const HUGE_HEADER: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x01, 0x86, 0xa0, 0x00, 0x01, 0x86, 0xa0, 0x08, 0x02, 0x00, 0x00, 0x00, 0x27, 0x30, 0x9c,
    0x9f, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xc9, 0xfe, 0x92, 0xef, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn a_valid_image_reports_its_format_size_and_bytes() {
    let path = scratch("valid", VALID);
    let report = check(&path).expect("a valid PNG checks");
    assert_eq!(report.format, "PNG");
    assert_eq!((report.width, report.height), (1, 1));
    assert_eq!(report.bytes, VALID.len() as u64);
    assert_eq!(
        report.to_string(),
        format!("ok: PNG 1x1, {} bytes", VALID.len())
    );
}

#[test]
fn a_huge_header_refuses_before_any_decode() {
    // 69 bytes on disk claiming 10 gigapixels: the refusal names the size,
    // and the file's own size proves nothing was allocated for it.
    let path = scratch("huge", HUGE_HEADER);
    let error = check(&path).expect_err("a huge header must refuse");
    assert!(
        error.contains("100000x100000"),
        "refusal names the claimed size: {error}"
    );
}

#[test]
fn a_corrupt_image_refuses() {
    let path = scratch("corrupt", b"this is not an image");
    let error = check(&path).expect_err("text must refuse");
    assert!(
        error.contains("not an image"),
        "unexpected refusal: {error}"
    );
}

#[test]
fn a_truncated_png_refuses() {
    let path = scratch("truncated", &VALID[..40]);
    let error = check(&path).expect_err("a truncated PNG must refuse");
    assert!(
        error.contains("cannot decode") || error.contains("truncated") || error.contains("corrupt"),
        "unexpected refusal: {error}"
    );
}

#[test]
fn a_fifo_refuses_without_hanging() {
    let dir = std::env::temp_dir().join(format!("scootbg-check-fifo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    let fifo = dir.join("image.png");
    let _ = std::fs::remove_file(&fifo);
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo failed");
    let error = check(&fifo).expect_err("a FIFO must refuse");
    assert!(
        error.contains("not a regular file"),
        "unexpected refusal: {error}"
    );
}

#[test]
fn a_symlink_refuses() {
    let real = scratch("symlink-real", VALID);
    let link = real.with_file_name("link.png");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&real, &link).expect("a symlink");
    let error = check(&link).expect_err("a symlink must refuse");
    assert!(error.contains("a symlink"), "unexpected refusal: {error}");
}

#[test]
fn a_directory_refuses() {
    let dir = std::env::temp_dir().join(format!("scootbg-check-dir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    let error = check(&dir).expect_err("a directory must refuse");
    assert!(
        error.contains("not a regular file"),
        "unexpected refusal: {error}"
    );
}

#[test]
fn a_missing_file_refuses() {
    let missing = std::env::temp_dir().join(format!(
        "scootbg-check-missing-{}-absent.png",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&missing);
    let error = check(&missing).expect_err("a missing file must refuse");
    assert!(error.contains("cannot stat"), "unexpected refusal: {error}");
}
