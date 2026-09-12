//! Reading the framebuffer back out as a PNG.
//!
//! With CPU rendering this is a copy of memory flexwm already has, which is
//! what makes screenshots cheap enough for an agent to take constantly.

use flexwm_ipc::Screenshot;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::{Bind, ExportMem};
use smithay::utils::Rectangle;

use super::State;
use super::headless::Backend;

impl State {
    /// Renders anything outstanding, then captures the screen.
    pub fn screenshot(&mut self) -> Result<Screenshot, String> {
        self.render();
        let Some(mut backend) = self.backend.take() else {
            return Err("no backend to capture".into());
        };
        let captured = capture(&mut backend);
        self.backend = Some(backend);
        captured
    }
}

fn capture(backend: &mut Backend) -> Result<Screenshot, String> {
    let (width, height) = backend.size;
    let region = Rectangle::from_size((width, height).into());
    let Backend {
        renderer, image, ..
    } = backend;

    let framebuffer = renderer.bind(image).map_err(|e| e.to_string())?;
    let mapping = renderer
        .copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)
        .map_err(|e| e.to_string())?;
    let pixels = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;

    // Argb8888 is little-endian BGRA in memory; PNG wants RGBA. This has to
    // be a copy rather than an in-place swap: map_texture's contract is a
    // read-only view into the renderer's own mapping (see its doc comment
    // in Smithay), not a buffer we're allowed to write back into.
    let mut rgba = Vec::with_capacity(pixels.len());
    for pixel in pixels.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }

    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // The default (Balanced) optimizes for file size; a screenshot here is a
    // transient thing an agent might pull every render tick, not an asset to
    // store, so latency matters more than shaving off a few KB.
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;

    Ok(Screenshot {
        width: width as u32,
        height: height as u32,
        png,
    })
}
