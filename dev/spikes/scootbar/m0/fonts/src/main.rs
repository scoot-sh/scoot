//! Throwaway spike: load one font file, fill a glyph cache with printable
//! ASCII at 1x and a fractional scale, draw one clock line from the cache,
//! and report time and memory. Exactly one engine feature is enabled per
//! build (none gives the base binary, for size deltas).
//!
//! Usage: sb-font-spike FONT [PX] [SCALES] [DUMP_DIR] [IDLE_SECS]
//!   PX       em size at 1x, in pixels (default 15)
//!   SCALES   comma-separated (default 1,1.5)
//!   DUMP_DIR write line-<scale>.pgm there ("-" for none)
//!   IDLE_SECS sleep, then report memory again (default 0)
//! Env: SB_HINT=1 turns swash hinting on.

use std::collections::HashMap;
use std::time::Instant;

struct Glyph {
    w: u32,
    h: u32,
    left: i32,
    /// Distance from the baseline up to the bitmap's top row.
    top: i32,
    advance: f32,
    cov: Vec<u8>,
}

trait Engine {
    fn glyph_id(&self, ch: char) -> u16;
    fn raster(&mut self, gid: u16, px: f32) -> Glyph;
}

fn mem() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let rollup = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    let get = |s: &str, k: &str| {
        s.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("?")
            .to_string()
    };
    format!(
        "rss={} pss={} anon={} file={} hwm={}",
        get(&status, "VmRSS:"),
        get(&rollup, "Pss:"),
        get(&status, "RssAnon:"),
        get(&status, "RssFile:"),
        get(&status, "VmHWM:")
    )
}

/// The font bytes for the process's lifetime, either heap or a mapping.
fn load(path: &str) -> &'static [u8] {
    #[cfg(feature = "mmap")]
    {
        let fd = rustix::fs::open(path, rustix::fs::OFlags::RDONLY, rustix::fs::Mode::empty())
            .expect("open font");
        let len = rustix::fs::fstat(&fd).expect("fstat").st_size as usize;
        // SAFETY: a private read-only mapping of a font file nobody rewrites
        // during the spike; it lives for the rest of the process.
        let ptr = unsafe {
            rustix::mm::mmap(
                std::ptr::null_mut(),
                len,
                rustix::mm::ProtFlags::READ,
                rustix::mm::MapFlags::PRIVATE,
                &fd,
                0,
            )
            .expect("mmap")
        };
        // SAFETY: the mapping above is `len` readable bytes and never unmapped.
        unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) }
    }
    #[cfg(not(feature = "mmap"))]
    {
        Box::leak(std::fs::read(path).expect("read font").into_boxed_slice())
    }
}

#[cfg(feature = "fontdue")]
mod eng {
    use super::*;
    pub struct E(fontdue::Font);
    pub fn new(data: &'static [u8]) -> E {
        E(
            fontdue::Font::from_bytes(data, fontdue::FontSettings::default())
                .expect("fontdue parse"),
        )
    }
    impl Engine for E {
        fn glyph_id(&self, ch: char) -> u16 {
            self.0.lookup_glyph_index(ch)
        }
        fn raster(&mut self, gid: u16, px: f32) -> Glyph {
            let (m, cov) = self.0.rasterize_indexed(gid, px);
            Glyph {
                w: m.width as u32,
                h: m.height as u32,
                left: m.xmin,
                top: m.ymin + m.height as i32,
                advance: m.advance_width,
                cov,
            }
        }
    }
}

#[cfg(feature = "ab_glyph")]
mod eng {
    use super::*;
    use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, point};
    pub struct E(FontRef<'static>);
    pub fn new(data: &'static [u8]) -> E {
        E(FontRef::try_from_slice(data).expect("ab_glyph parse"))
    }
    impl Engine for E {
        fn glyph_id(&self, ch: char) -> u16 {
            self.0.glyph_id(ch).0
        }
        fn raster(&mut self, gid: u16, px: f32) -> Glyph {
            // ab_glyph's PxScale is ascent-to-descent height, not the em.
            let upem = self.0.units_per_em().unwrap_or(1000.0);
            let scale = PxScale::from(px * self.0.height_unscaled() / upem);
            let id = GlyphId(gid);
            let advance = self.0.as_scaled(scale).h_advance(id);
            let glyph = id.with_scale_and_position(scale, point(0.0, 0.0));
            match self.0.outline_glyph(glyph) {
                Some(og) => {
                    let b = og.px_bounds();
                    let (w, h) = (b.width() as u32, b.height() as u32);
                    let mut cov = vec![0u8; (w * h) as usize];
                    og.draw(|x, y, c| {
                        if let Some(p) = cov.get_mut((y * w + x) as usize) {
                            *p = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                        }
                    });
                    Glyph {
                        w,
                        h,
                        left: b.min.x as i32,
                        top: -(b.min.y as i32),
                        advance,
                        cov,
                    }
                }
                None => Glyph {
                    w: 0,
                    h: 0,
                    left: 0,
                    top: 0,
                    advance,
                    cov: Vec::new(),
                },
            }
        }
    }
}

#[cfg(feature = "swash")]
mod eng {
    use super::*;
    use swash::FontRef;
    use swash::scale::{Render, ScaleContext, Source};
    use swash::zeno::Format;
    pub struct E {
        font: FontRef<'static>,
        ctx: ScaleContext,
        hint: bool,
    }
    pub fn new(data: &'static [u8]) -> E {
        E {
            font: FontRef::from_index(data, 0).expect("swash parse"),
            ctx: ScaleContext::new(),
            hint: std::env::var_os("SB_HINT").is_some_and(|v| v == "1"),
        }
    }
    impl Engine for E {
        fn glyph_id(&self, ch: char) -> u16 {
            self.font.charmap().map(ch)
        }
        fn raster(&mut self, gid: u16, px: f32) -> Glyph {
            let advance = self.font.glyph_metrics(&[]).scale(px).advance_width(gid);
            let mut scaler = self.ctx.builder(self.font).size(px).hint(self.hint).build();
            match Render::new(&[Source::Outline])
                .format(Format::Alpha)
                .render(&mut scaler, gid)
            {
                Some(img) => Glyph {
                    w: img.placement.width,
                    h: img.placement.height,
                    left: img.placement.left,
                    top: img.placement.top,
                    advance,
                    cov: img.data,
                },
                None => Glyph {
                    w: 0,
                    h: 0,
                    left: 0,
                    top: 0,
                    advance,
                    cov: Vec::new(),
                },
            }
        }
    }
}

#[cfg(not(any(feature = "fontdue", feature = "ab_glyph", feature = "swash")))]
mod eng {
    use super::*;
    pub struct E(&'static [u8]);
    pub fn new(data: &'static [u8]) -> E {
        E(data)
    }
    impl Engine for E {
        fn glyph_id(&self, ch: char) -> u16 {
            ch as u16 ^ self.0.first().copied().unwrap_or(0) as u16
        }
        fn raster(&mut self, _gid: u16, px: f32) -> Glyph {
            Glyph {
                w: 0,
                h: 0,
                left: 0,
                top: 0,
                advance: px,
                cov: Vec::new(),
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: sb-font-spike FONT [PX] [SCALES] [DUMP_DIR] [IDLE_SECS]");
    let px: f32 = args.get(2).map_or(15.0, |s| s.parse().expect("PX"));
    let scales: Vec<f32> = args
        .get(3)
        .map_or("1,1.5", String::as_str)
        .split(',')
        .map(|s| s.parse().expect("SCALES"))
        .collect();
    let dump = args.get(4).filter(|d| *d != "-");
    let idle: u64 = args.get(5).map_or(0, |s| s.parse().expect("IDLE_SECS"));

    println!("start {}", mem());
    let t = Instant::now();
    let data = load(path);
    let mut e = eng::new(data);
    let load_us = t.elapsed().as_micros();
    println!("loaded {} load_us={load_us}", mem());

    let set: Vec<char> = (0x20u8..=0x7e).map(char::from).collect();
    let mut cache: HashMap<(u16, u32), Glyph> = HashMap::new();
    for &s in &scales {
        let size = px * s;
        let t = Instant::now();
        for &ch in &set {
            let gid = e.glyph_id(ch);
            cache
                .entry((gid, size.to_bits()))
                .or_insert_with(|| e.raster(gid, size));
        }
        let fill_us = t.elapsed().as_micros();
        let bytes: usize = cache.values().map(|g| g.cov.len()).sum();
        let sum: u64 = cache
            .values()
            .flat_map(|g| g.cov.iter())
            .map(|&c| u64::from(c))
            .sum();
        println!(
            "scale={s} px={size} fill_us={fill_us} glyphs={} bitmap_bytes={bytes} coverage_sum={sum}",
            cache.len()
        );
    }
    println!("filled {}", mem());

    // One clock line from the cache: the per-minute steady-state cost.
    let line = "Tue 29 Sep 14:07";
    for &s in &scales {
        let size = px * s;
        let asc = (size * 1.2).ceil() as i32;
        let height = (size * 1.6).ceil() as usize;
        let width: usize = line
            .chars()
            .map(|c| cache[&(e.glyph_id(c), size.to_bits())].advance)
            .sum::<f32>()
            .ceil() as usize
            + 4;
        let mut buf = vec![0u8; width * height];
        let t = Instant::now();
        let mut pen = 2.0f32;
        for c in line.chars() {
            let g = &cache[&(e.glyph_id(c), size.to_bits())];
            let x0 = pen.round() as i32 + g.left;
            let y0 = asc - g.top;
            for gy in 0..g.h as i32 {
                for gx in 0..g.w as i32 {
                    let (x, y) = (x0 + gx, y0 + gy);
                    if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                        let d = &mut buf[y as usize * width + x as usize];
                        *d = (*d).max(g.cov[(gy as u32 * g.w + gx as u32) as usize]);
                    }
                }
            }
            pen += g.advance;
        }
        let draw_ns = t.elapsed().as_nanos();
        println!("draw scale={s} {width}x{height} draw_ns={draw_ns}");
        if let Some(dir) = dump {
            let mut out = format!("P5\n{width} {height}\n255\n").into_bytes();
            out.extend(buf.iter().map(|&v| 255 - v));
            std::fs::write(format!("{dir}/line-{s}.pgm"), out).expect("write pgm");
        }
    }
    println!("final {}", mem());
    if idle > 0 {
        std::thread::sleep(std::time::Duration::from_secs(idle));
        println!("idle {}", mem());
    }
}
