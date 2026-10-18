//! `launch-proto`: the scootlaunch go/no-go throwaway. A layer-shell
//! overlay panel with exclusive keyboard, `#[path]`-including scootbar's
//! drawing and text sources, decoding keys through `xkbcommon`. Deleted
//! after measuring, like the keyboard spike.

mod draw;
mod index;
mod shm;
mod stub;
mod ui;

// scootbar sources compiled unchanged at the crate root, so their
// `crate::color` / `crate::paint` / `crate::icon` / `crate::text` /
// `crate::modules` references resolve: the ticket's `#[path]` pattern
// (`crates/scootbar/fuzz/Cargo.toml`'s fuzz targets do the same).
//
// Included: drawing (`paint`), colors (`color`), text + glyph cache
// (`text`), font loading (`font`), the look tokens (`theme`), and the icon
// modules `text` reaches (`icon/cache`, `path`, `raster`, `sample`).
// `icon/image` and `icon/tray` compile out: they sit behind the
// `icon-image`/`tray` features, which this crate never sets.

#[rustfmt::skip]
#[path = "../../w9-launch/crates/scootbar/src/color.rs"]
pub mod color;

#[rustfmt::skip]
#[path = "../../w9-launch/crates/scootbar/src/paint.rs"]
pub mod paint;

#[rustfmt::skip]
#[path = "../../w9-launch/crates/scootbar/src/icon/mod.rs"]
pub mod icon;

#[rustfmt::skip]
#[path = "../../w9-launch/crates/scootbar/src/text.rs"]
pub mod text;

#[rustfmt::skip]
#[path = "../../w9-launch/crates/scootbar/src/font.rs"]
pub mod font;

#[path = "sb_modules.rs"]
pub mod modules;

#[rustfmt::skip]
#[path = "../../w9-launch/crates/scootbar/src/theme.rs"]
pub mod theme;

use std::path::PathBuf;
use std::time::Instant;

fn usage() -> ! {
    eprintln!(
        "usage:
  launch-proto [--entries N] [--apps-dir D]... [--font PATH] [--trace FILE]
               [--stub-sock PATH | --spawn-stub] [--quit-after-ms N]
  launch-proto --stub --app-id ID [--sock PATH] [--quit-after-ms N] [--trace FILE]
  launch-proto --bench [--entries N] [--apps-dir D]... [--iters N] [--font PATH]"
    );
    std::process::exit(2);
}

fn main() -> ! {
    let mut args = std::env::args().skip(1);
    let mut mode = "panel";
    let mut entries_n: usize = 0;
    let mut apps_dirs: Vec<PathBuf> = Vec::new();
    let mut font: Option<PathBuf> = None;
    let mut trace: Option<PathBuf> = None;
    let mut stub_sock: Option<PathBuf> = None;
    let mut spawn_stub = false;
    let mut use_focus_serial = false;
    let mut quit_after_ms: Option<u64> = None;
    let mut app_id = "w9.launch.stub".to_string();
    let mut sock: Option<PathBuf> = None;
    let mut iters: usize = 20;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--stub" => mode = "stub",
            "--bench" => mode = "bench",
            "--entries" => entries_n = args.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage()),
            "--apps-dir" => apps_dirs.push(args.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            "--font" => font = Some(args.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            "--trace" => trace = Some(args.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            "--stub-sock" => stub_sock = Some(args.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            "--selftest-compose" => mode = "selftest-compose",
            "--spawn-stub" => spawn_stub = true,
            "--use-focus-serial" => use_focus_serial = true,
            "--quit-after-ms" => quit_after_ms = Some(args.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage())),
            "--app-id" => app_id = args.next().unwrap_or_else(|| usage()),
            "--sock" => sock = Some(args.next().map(PathBuf::from).unwrap_or_else(|| usage())),
            "--iters" => iters = args.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage()),
            _ => usage(),
        }
    }

    match mode {
        "stub" => {
            let code = stub::run(stub::StubOpts { app_id, sock, quit_after_ms }, trace);
            std::process::exit(code);
        }
        "selftest-compose" => {
            let code = ui::selftest_compose();
            std::process::exit(code);
        }
        "bench" => {
            let code = bench(entries_n, &apps_dirs, iters, font);
            std::process::exit(code);
        }
        _ => {
            let mut entries = index::scan(&apps_dirs);
            let have = entries.len();
            index::pad(&mut entries, entries_n.max(have));
            if entries_n > 0 {
                entries.truncate(entries_n);
            }
            eprintln!("launch-proto: {} entries", entries.len());
            let font_path = font.unwrap_or_else(|| {
                crate::font::find(None)
                    .unwrap_or_else(|e| {
                        eprintln!("no font: {e:?}");
                        std::process::exit(1);
                    })
                    .path
            });
            let code = ui::run(
                ui::Opts { font: font_path, trace, stub_sock, spawn_stub, quit_after_ms, use_focus_serial },
                entries,
            );
            std::process::exit(code);
        }
    }
}

/// Headless CPU path: index build, then full rescore + full draw per
/// simulated keystroke of a fixed query, no compositor. The number the
/// live row cannot isolate.
fn bench(entries_n: usize, apps_dirs: &[PathBuf], iters: usize, font: Option<PathBuf>) -> i32 {
    let t = Instant::now();
    let mut entries = index::scan(apps_dirs);
    let scan_us = t.elapsed().as_micros();
    let real = entries.len();
    let have = real;
    index::pad(&mut entries, entries_n.max(have));
    if entries_n > 0 {
        entries.truncate(entries_n);
    }
    let font_path = font.unwrap_or_else(|| {
        crate::font::find(None)
            .unwrap_or_else(|e| {
                eprintln!("no font: {e:?}");
                std::process::exit(1);
            })
            .path
    });
    let mut text =
        crate::font::text(Some(font_path.as_path()), &[]).unwrap_or_else(|e| {
            eprintln!("font load: {e:?}");
            std::process::exit(1);
        });
    // A query that matches deep into a big list, then narrows.
    let strokes = ["f", "fi", "fir", "fire", "firef", "firefo", "firefox"];
    let mut pixels = vec![0u8; (draw::W * draw::H * 4) as usize];
    let mut rescore: Vec<u128> = Vec::new();
    let mut redraw: Vec<u128> = Vec::new();
    let mut query = String::new();
    for _ in 0..iters {
        query.clear();
        for s in &strokes {
            query.push_str(&s[query.len()..]);
            let t = Instant::now();
            let order = index::rank(&entries, &query).order;
            rescore.push(t.elapsed().as_nanos());
            let t = Instant::now();
            draw::draw_panel(&mut pixels, &mut text, &query, &order, &entries, 0);
            redraw.push(t.elapsed().as_nanos());
        }
    }
    rescore.sort_unstable();
    redraw.sort_unstable();
    let median = |v: &[u128]| v[v.len() / 2] as f64 / 1000.0;
    let mean = |v: &[u128]| v.iter().sum::<u128>() as f64 / v.len() as f64 / 1000.0;
    println!(
        "BENCH entries={} real={} iters={} strokes={}",
        entries.len(),
        real,
        iters,
        strokes.len()
    );
    println!("BENCH scan_us={scan_us}");
    println!("BENCH rescore_mean_us={:.1} rescore_median_us={:.1}", mean(&rescore), median(&rescore));
    println!("BENCH redraw_mean_us={:.1} redraw_median_us={:.1}", mean(&redraw), median(&redraw));
    0
}
