//! The GPU half of `--nested`'s dma-buf presentation (`nested/gpu.rs`), on
//! real buffers: a frame drawn into the GLES render target, copied on the
//! GPU into a host buffer allocated the way the nested backend allocates
//! them, and read by a *second* renderer the way a host would -- imported as
//! a texture and drawn.
//!
//! What this pins that no unit test can: that the device's GBM will
//! allocate a buffer this renderer can render into and blit to, that the
//! copy lands the right way up (the `flipped()` trap `read_back`'s doc
//! describes applies to a blit just as much), and that another EGL display
//! on the same device sees exactly the bytes a capture of the render target
//! does -- i.e. the host shows what `scoot msg screenshot` says it shows.
//!
//! Needs a DRM device (render node or primary node) GBM can allocate on.
//! Where this machine has none (CI, a GPU-less container) the tests say so on
//! stderr and assert nothing -- the same trade `dmabuf/tests/layouts.rs`
//! makes -- while the pure halves (`nested/gpu/tests.rs`) run everywhere.

use smithay::backend::allocator::Buffer as _;
use smithay::backend::allocator::gbm::GbmAllocator;
use smithay::backend::drm::{DrmDeviceFd, DrmNode, NodeType};
use smithay::backend::renderer::Frame as _;

use super::super::super::nested::gpu::feedback::{self, Choice, HostFeedback, HostTranche};
use super::super::super::nested::gpu::{allocate, open_node};
use super::*;

/// Opens GBM on the renderer's device -- render node first, then its primary
/// node -- and chooses a host format the way `negotiate` does, against a
/// host that is this same renderer (it lists exactly what the renderer can
/// render into, on its own device). `Err` names why this machine cannot.
fn allocator_for(backend: &Backend) -> Result<(GbmAllocator<DrmDeviceFd>, Choice), String> {
    let device = backend
        .render_node()
        .ok_or("the renderer's EGL device names no DRM node")?;
    let formats = backend
        .dmabuf_render_formats()
        .ok_or("not a GLES backend")?;
    let host = HostFeedback {
        main_device: device,
        tranches: vec![HostTranche {
            device,
            scanout: false,
            formats: formats
                .iter()
                .map(|format| (format.code as u32, u64::from(format.modifier)))
                .collect(),
        }],
    };
    let choice = feedback::choose(
        &host,
        |dev| feedback::same_drm_device(dev, device),
        |format| formats.contains(&format),
    )
    .ok_or("the renderer renders into no Argb8888/Xrgb8888 layout")?;
    let render = DrmNode::from_dev_id(device).map_err(|error| error.to_string())?;
    let mut nodes = vec![render];
    if let Some(Ok(primary)) = render.node_with_type(NodeType::Primary) {
        nodes.push(primary);
    }
    let mut failures = Vec::new();
    for node in nodes {
        match open_node(&node).and_then(|mut allocator| {
            allocate(&mut allocator, &choice, 8, 8)?;
            Ok(allocator)
        }) {
            Ok(allocator) => return Ok((allocator, choice)),
            Err(error) => failures.push(format!("{node}: {error}")),
        }
    }
    Err(failures.join("; "))
}

/// A GLES backend holding the orientation scene: green over red.
fn marker_backend() -> Backend {
    let output = test_output(MARKER_CANVAS, MARKER_CANVAS);
    let mut backend = Backend::new(
        &output,
        MARKER_CANVAS,
        MARKER_CANVAS,
        RendererKind::Gles,
        ScanoutHandoff::default(),
        None,
    )
    .expect("a GLES backend");
    let Backend {
        pipeline, damage, ..
    } = &mut backend;
    let Pipeline::Gles(gpu) = pipeline else {
        unreachable!("built as GLES");
    };
    draw_markers(&mut gpu.renderer, &mut gpu.buffer, damage);
    backend
}

/// What a host on the same device sees of `dmabuf`: a second GLES backend,
/// pinned to `pin`, imports it as a texture and draws it over its whole
/// target, and that target is read back.
fn as_the_host_sees_it(dmabuf: &Dmabuf, pin: GlesDevice) -> Vec<u8> {
    let size = dmabuf.size();
    let output = test_output(size.w, size.h);
    let mut host = Backend::new(
        &output,
        size.w,
        size.h,
        RendererKind::Gles,
        ScanoutHandoff::default(),
        Some(pin),
    )
    .expect("a second GLES backend on the same device");
    {
        let Pipeline::Gles(gpu) = &mut host.pipeline else {
            unreachable!("built as GLES");
        };
        let texture = ImportDma::import_dmabuf(&mut gpu.renderer, dmabuf, None)
            .expect("the host stand-in imports the buffer");
        let full = Rectangle::from_size((size.w, size.h).into());
        let mut framebuffer = gpu.renderer.bind(&mut gpu.buffer).expect("a framebuffer");
        let mut frame = gpu
            .renderer
            .render(&mut framebuffer, (size.w, size.h).into(), Transform::Normal)
            .expect("a frame");
        frame
            .clear([0.0, 0.0, 1.0, 1.0].into(), &[full])
            .expect("a clear");
        frame
            .render_texture_at(
                &texture,
                (0, 0).into(),
                1,
                1.0,
                Transform::Normal,
                &[full],
                &[],
                1.0,
            )
            .expect("the texture draws");
        frame
            .finish()
            .expect("the frame finishes")
            .wait()
            .expect("and lands");
    }
    host.capture(<[u8]>::to_vec)
        .expect("the host target reads back")
}

/// The whole path, on real buffers: a copied frame reads back on another
/// renderer byte-for-byte as a capture of the render target does, the right
/// way up.
#[test]
fn a_frame_copied_into_a_host_buffer_is_what_another_renderer_sees() {
    let mut backend = marker_backend();
    let (mut allocator, choice) = match allocator_for(&backend) {
        Ok(found) => found,
        Err(reason) => {
            eprintln!(
                "a_frame_copied_into_a_host_buffer_is_what_another_renderer_sees: skipped -- \
                 no GBM device serves this renderer ({reason})"
            );
            return;
        }
    };
    let captured = backend.capture(<[u8]>::to_vec).expect("a capture");
    let mut dmabuf = allocate(&mut allocator, &choice, MARKER_CANVAS, MARKER_CANVAS)
        .expect("a host buffer at the frame's size");
    backend
        .copy_frame_into(&mut dmabuf)
        .expect("the frame copies into the host buffer");

    let pin = backend.gles_device().expect("a GLES device");
    let seen = as_the_host_sees_it(&dmabuf, pin);
    assert_eq!(
        pixel(&seen, MARKER_CANVAS, 0, 0),
        GREEN_BGRA,
        "the host must see the logical top-left at the top-left ({choice:?})"
    );
    assert_eq!(
        pixel(&seen, MARKER_CANVAS, 0, MARKER_CANVAS - 1),
        RED_BGRA,
        "the host must see the logical bottom at the bottom ({choice:?})"
    );
    let differing = captured
        .chunks_exact(4)
        .zip(seen.chunks_exact(4))
        .filter(|(left, right)| left != right)
        .count();
    assert_eq!(
        differing, 0,
        "the host sees {differing} pixels other than a capture of the render target does"
    );
}

/// Copying a frame out leaves the render target -- what every capture reads
/// -- exactly as it was: presenting by dma-buf changes nothing a screenshot
/// or a screencopy client sees.
#[test]
fn copying_a_frame_out_leaves_what_captures_read_untouched() {
    let mut backend = marker_backend();
    let (mut allocator, choice) = match allocator_for(&backend) {
        Ok(found) => found,
        Err(reason) => {
            eprintln!(
                "copying_a_frame_out_leaves_what_captures_read_untouched: skipped -- no GBM \
                 device serves this renderer ({reason})"
            );
            return;
        }
    };
    let before = backend.capture(<[u8]>::to_vec).expect("a capture");
    for _ in 0..3 {
        let mut dmabuf =
            allocate(&mut allocator, &choice, MARKER_CANVAS, MARKER_CANVAS).expect("a host buffer");
        backend
            .copy_frame_into(&mut dmabuf)
            .expect("the frame copies");
    }
    let after = backend.capture(<[u8]>::to_vec).expect("a capture");
    assert!(
        before == after,
        "a copy out must not change the render target"
    );
}

/// The startup probe's shape: a host buffer *smaller* than the frame takes
/// the overlapping corner rather than failing, so the probe can be small
/// whatever size the session starts at.
#[test]
fn a_smaller_host_buffer_takes_the_top_left_corner() {
    let mut backend = marker_backend();
    let (mut allocator, choice) = match allocator_for(&backend) {
        Ok(found) => found,
        Err(reason) => {
            eprintln!(
                "a_smaller_host_buffer_takes_the_top_left_corner: skipped -- no GBM device \
                 serves this renderer ({reason})"
            );
            return;
        }
    };
    let side = MARKER_CANVAS / 4;
    let mut dmabuf = allocate(&mut allocator, &choice, side, side).expect("a small host buffer");
    backend
        .copy_frame_into(&mut dmabuf)
        .expect("a corner copies");
    let pin = backend.gles_device().expect("a GLES device");
    let seen = as_the_host_sees_it(&dmabuf, pin);
    assert_eq!(pixel(&seen, side, 0, 0), GREEN_BGRA);
    assert_eq!(pixel(&seen, side, side - 1, side - 1), GREEN_BGRA);
}

/// pixman has no GPU buffer to hand over: it answers no render formats, so
/// `negotiate` never picks the dma-buf path for it, and a copy is refused
/// rather than attempted.
#[test]
fn pixman_offers_no_dma_buf_presentation() {
    let output = test_output(16, 16);
    let backend = Backend::new(
        &output,
        16,
        16,
        RendererKind::Pixman,
        ScanoutHandoff::default(),
        None,
    )
    .expect("a pixman backend");
    assert!(backend.dmabuf_render_formats().is_none());
    assert!(backend.exceeds_max_target(65535, 65535).is_none());
}
