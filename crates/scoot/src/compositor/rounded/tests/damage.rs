//! The opaque-region cut on a damage-tracked path: what is below the cut
//! must repaint when it changes.
//!
//! The unit test in `super` pins that [`Rounded`] shrinks the opaque
//! region; the pixel suites pin that the draw follows it. Neither runs
//! through Smithay's damage tracker, which is exactly what trusts the
//! opaque claim: an opaque region reaching past the clip lets the tracker
//! skip what is below pixels the draw leaves untouched, so the stale
//! pixels only show on a backend that tracks damage (`--tty`, which
//! passes a real buffer age) -- `--headless` and `--nested` pass age 0
//! and repaint fully either way. This drives a real [`PixmanRenderer`]
//! through a real [`OutputDamageTracker`] at age 1 and asserts on
//! framebuffer bytes: the corner cut and the geometry-bounds cut each get
//! a probe pixel, so neutering either half of the opaque cut fails this
//! test while leaving every headless pixel suite green.

use crate::compositor::rounded::Rounded;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::UnderlyingStorage;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer as BufferSpace, Physical, Rectangle, Scale, Transform};

const SIZE: i32 = 120;
/// The window geometry, in output pixels: a 10px margin all around the
/// 120px surface never reaches the screen.
fn clip() -> Rectangle<i32, Physical> {
    Rectangle::new((10, 10).into(), (100, 100).into())
}
const RADIUS: i32 = 12;

const BLUE: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const YELLOW: [u8; 4] = [0, 255, 255, 255];

/// Front-to-back, like the frame path gathers: the rounded window first,
/// what is below it second.
enum Frame {
    Window(Rounded<SolidColorRenderElement>),
    Below(SolidColorRenderElement),
}

impl Element for Frame {
    fn id(&self) -> &Id {
        match self {
            Frame::Window(window) => window.id(),
            Frame::Below(below) => below.id(),
        }
    }

    fn current_commit(&self) -> CommitCounter {
        match self {
            Frame::Window(window) => window.current_commit(),
            Frame::Below(below) => below.current_commit(),
        }
    }

    fn src(&self) -> Rectangle<f64, BufferSpace> {
        match self {
            Frame::Window(window) => window.src(),
            Frame::Below(below) => below.src(),
        }
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        match self {
            Frame::Window(window) => window.geometry(scale),
            Frame::Below(below) => below.geometry(scale),
        }
    }

    fn transform(&self) -> Transform {
        match self {
            Frame::Window(window) => window.transform(),
            Frame::Below(below) => below.transform(),
        }
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        match self {
            Frame::Window(window) => window.damage_since(scale, commit),
            Frame::Below(below) => below.damage_since(scale, commit),
        }
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        match self {
            Frame::Window(window) => window.opaque_regions(scale),
            Frame::Below(below) => below.opaque_regions(scale),
        }
    }

    fn alpha(&self) -> f32 {
        match self {
            Frame::Window(window) => window.alpha(),
            Frame::Below(below) => below.alpha(),
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Frame::Window(window) => window.kind(),
            Frame::Below(below) => below.kind(),
        }
    }
}

impl RenderElement<PixmanRenderer> for Frame {
    fn draw(
        &self,
        frame: &mut <PixmanRenderer as smithay::backend::renderer::RendererSuper>::Frame<'_, '_>,
        src: Rectangle<f64, BufferSpace>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), <PixmanRenderer as smithay::backend::renderer::RendererSuper>::Error> {
        match self {
            Frame::Window(window) => RenderElement::<PixmanRenderer>::draw(
                window,
                frame,
                src,
                dst,
                damage,
                opaque_regions,
                cache,
            ),
            Frame::Below(below) => RenderElement::<PixmanRenderer>::draw(
                below,
                frame,
                src,
                dst,
                damage,
                opaque_regions,
                cache,
            ),
        }
    }

    fn underlying_storage(&self, renderer: &mut PixmanRenderer) -> Option<UnderlyingStorage<'_>> {
        match self {
            Frame::Window(window) => window.underlying_storage(renderer),
            Frame::Below(below) => below.underlying_storage(renderer),
        }
    }
}

/// Renders `elements` at `age` into a fresh framebuffer and hands back the
/// tracker's damage report and the read-back bytes.
fn render_frame(
    tracker: &mut OutputDamageTracker,
    renderer: &mut PixmanRenderer,
    age: usize,
    elements: &[Frame],
) -> (Option<Vec<Rectangle<i32, Physical>>>, Vec<u8>) {
    let mut image = renderer
        .create_buffer(Fourcc::Argb8888, (SIZE, SIZE).into())
        .expect("an image");
    let mut framebuffer = renderer.bind(&mut image).expect("a framebuffer");
    let rendered = tracker
        .render_output(
            renderer,
            &mut framebuffer,
            age,
            elements,
            [0.0, 0.0, 0.0, 1.0],
        )
        .expect("a rendered frame");
    let damage = rendered.damage.cloned();
    drop(rendered);
    let region = Rectangle::new((0, 0).into(), (SIZE, SIZE).into());
    let mapping = renderer
        .copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)
        .expect("a framebuffer readback");
    let pixels = renderer
        .map_texture(&mapping)
        .expect("mapped pixels")
        .to_vec();
    (damage, pixels)
}

fn at(pixels: &[u8], x: i32, y: i32) -> [u8; 4] {
    let index = ((y * SIZE + x) * 4) as usize;
    pixels[index..index + 4]
        .try_into()
        .expect("four bytes per pixel")
}

fn window_and_below(
    window_buffer: &SolidColorBuffer,
    below_buffer: &SolidColorBuffer,
) -> [Frame; 2] {
    let surface =
        SolidColorRenderElement::from_buffer(window_buffer, (0, 0), 1.0, 1.0, Kind::Unspecified);
    let window = Rounded::new(surface, clip(), RADIUS).clipped_to(clip());
    let below =
        SolidColorRenderElement::from_buffer(below_buffer, (0, 0), 1.0, 1.0, Kind::Unspecified);
    [Frame::Window(window), Frame::Below(below)]
}

/// What is below the cut repaints when it changes, on a damage-tracked
/// path: the corner probe pins the corner-square cut, the margin probe
/// pins the geometry-bounds cut. Neuter check: returning the inner opaque
/// region uncut (or skipping just the corner-square subtraction) fails the
/// first frame's corner assert -- the cut corner reads clear instead of
/// green; skipping just the geometry-bounds clip fails the first frame's
/// margin assert instead.
#[test]
fn what_is_below_the_cut_repaints_when_it_changes() {
    let window_buffer = SolidColorBuffer::new((SIZE, SIZE), [0.0, 0.0, 1.0, 1.0]);
    let mut below_buffer = SolidColorBuffer::new((SIZE, SIZE), [0.0, 1.0, 0.0, 1.0]);
    let mut tracker = OutputDamageTracker::new((SIZE, SIZE), 1.0, Transform::Normal);
    let mut renderer = PixmanRenderer::new().expect("a cpu renderer");

    let elements = window_and_below(&window_buffer, &below_buffer);
    let (damage, pixels) = render_frame(&mut tracker, &mut renderer, 0, &elements);
    assert!(damage.is_some(), "the first frame paints the whole output");
    // The cut shows what is below before anything changes: the corner is
    // inside the top-left corner square, the margin outside the geometry.
    assert_eq!(at(&pixels, 11, 11), GREEN, "the cut corner shows below");
    assert_eq!(at(&pixels, 5, 60), GREEN, "the margin shows below");
    assert_eq!(
        at(&pixels, 60, 60),
        BLUE,
        "the window middle stays the window"
    );
    assert_eq!(
        at(&pixels, 60, 10),
        BLUE,
        "the top edge middle stays the window"
    );

    // Below changes color without the window committing anything: its new
    // commit damages the whole canvas, and the corner and margin probes
    // must come from its repaint -- the window draws nothing there.
    below_buffer.set_color([1.0, 1.0, 0.0, 1.0]);
    let elements = window_and_below(&window_buffer, &below_buffer);
    let (damage, pixels) = render_frame(&mut tracker, &mut renderer, 1, &elements);
    assert!(
        damage.is_some(),
        "below changed, so the frame must carry damage"
    );
    assert_eq!(
        at(&pixels, 11, 11),
        YELLOW,
        "the cut corner repaints to what is below now"
    );
    assert_eq!(
        at(&pixels, 5, 60),
        YELLOW,
        "the margin repaints to what is below now"
    );
    assert_eq!(
        at(&pixels, 60, 60),
        BLUE,
        "the window middle stays the window"
    );
    assert_eq!(
        at(&pixels, 60, 10),
        BLUE,
        "the top edge middle stays the window"
    );
}
