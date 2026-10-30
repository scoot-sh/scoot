//! The panel's stage: the shadow behind it and the offscreen layers a
//! moving panel is composited from.
//!
//! A panel with no shadow and no motion never comes here: it paints
//! straight into the buffer, the bytes it always did. Otherwise the panel
//! is painted once into a [`Layer`] at its own origin (so none of the
//! panel's geometry changes), the shadow is painted under it into a second,
//! surface-sized scene, and [`Stage::present`] puts the scene into the
//! shm buffer, scaled and faded by the pose. A frame of an entrance or
//! fade is therefore one blit; the panel is repainted only when its
//! content changes.
//!
//! The scene is symmetric around the panel: a shadow's room is the larger
//! of its two sides on each axis, because the compositor centers the
//! surface and an uneven room would move the panel. The shadow is built in
//! device pixels for the scale it is drawn at and rebuilt only when that
//! changes. Nothing here allocates after the first frame at a size.

use scootui::color::Color;
use scootui::effects::ShadowStyle;
use scootui::paint::{Canvas, Layer, Shadow, ShadowSpec};
use scootui::region::Rect;

use crate::look::Look;
use crate::motion::{Motion, Pose, Slide};

/// The most pixels a layer holds here: 16 Mpx (64 MiB) is a panel larger
/// than a 4K output, which the shm buffer already has to hold.
const LAYER_PIXELS: usize = 16 * 1024 * 1024;

/// The room, in logical pixels per side, a shadow of `style` needs around
/// the panel: `(horizontal, vertical)`. Zero for no shadow.
pub fn room(style: ShadowStyle) -> (u32, u32) {
    let Some(spec) = style.spec() else {
        return (0, 0);
    };
    let pad = Shadow::new(spec).pad();
    (pad.left.max(pad.right), pad.top.max(pad.bottom))
}

/// `spec` for a surface drawn at `factor` device pixels per logical one.
fn scaled(spec: ShadowSpec, factor: f64) -> ShadowSpec {
    let by = |value: i32| (f64::from(value) * factor).round() as i32;
    ShadowSpec {
        offset_x: by(spec.offset_x),
        offset_y: by(spec.offset_y),
        blur: (f64::from(spec.blur) * factor).round().clamp(0.0, 4096.0) as u32,
        alpha: spec.alpha,
    }
}

/// Where the panel sits in this frame's scene, all in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scene {
    /// The shm buffer's size.
    pub surface: (u32, u32),
    /// The panel's size.
    pub panel: (u32, u32),
    /// The panel's top-left corner in the buffer.
    pub origin: (u32, u32),
    /// Device pixels per logical pixel.
    pub factor: f64,
    /// The panel's corner radius (the chrome's, before it is cut back to
    /// what the panel holds).
    pub radius: u32,
    /// What a glow is tinted with.
    pub tint: Color,
}

/// What a look's depth and motion add to one open: the room the surface
/// keeps around the panel, the stage the panel is composed on, and the
/// motion that moves it. [`Effects::off`] (what a look without a shadow or
/// motion gets) has no room, no stage and no motion, and the panel paints
/// straight into the buffer as it always did.
#[derive(Debug)]
pub struct Effects {
    /// The shadow's room, in logical pixels per side: `(horizontal,
    /// vertical)`.
    pub room: (u32, u32),
    /// The layers; `None` when neither a shadow nor motion is asked for.
    pub stage: Option<Stage>,
    /// The motion; inactive without a stage.
    pub motion: Motion,
    /// The scene last composed, so a frame that only advances a motion
    /// presents it again instead of repainting the panel.
    pub composed: Option<Scene>,
    /// The rows' generation at the last frame ([`crate::ui::Ui::generation`]):
    /// a selection that moved within the same rows slides, rows that
    /// changed under it do not.
    pub generation: u64,
    /// Where the highlight was drawn in the scene last composed: a frame
    /// repaints the panel when it is somewhere else, which includes the
    /// frame it comes to rest on.
    pub slide: Option<Slide>,
}

impl Effects {
    /// Nothing: the plain panel.
    pub fn off() -> Self {
        Self {
            room: (0, 0),
            stage: None,
            motion: Motion::new(scootui::motion::Timing::NONE),
            composed: None,
            generation: 0,
            slide: None,
        }
    }

    /// The effects `look` asks for.
    pub fn from_look(look: &Look) -> Self {
        let timing = look.timing();
        let room = room(look.shadow);
        if room == (0, 0) && !timing.active() {
            return Self::off();
        }
        Self {
            room,
            stage: Some(Stage::new(look.shadow)),
            motion: Motion::new(timing),
            composed: None,
            generation: 0,
            slide: None,
        }
    }
}

/// The room a surface of `surface` logical pixels actually has around a
/// panel of `panel` logical pixels when `wanted` was asked for: the
/// compositor may configure a smaller surface than requested (a tiny
/// output), and the shadow then shrinks to fit rather than the panel, which
/// never moves. The room stays symmetric, so the panel stays centered.
pub fn fit_room(wanted: (u32, u32), surface: (u32, u32), panel: (u32, u32)) -> (u32, u32) {
    let fit = |wanted: u32, surface: u32, panel: u32| wanted.min(surface.saturating_sub(panel) / 2);
    (
        fit(wanted.0, surface.0, panel.0),
        fit(wanted.1, surface.1, panel.1),
    )
}

/// What the shadow's tables were built for.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Built {
    factor: f64,
    panel: (u32, u32),
    radius: u32,
}

/// The layers and tables, kept for the whole open.
#[derive(Debug)]
pub struct Stage {
    style: ShadowStyle,
    shadow: Option<Shadow>,
    built: Option<Built>,
    panel: Layer,
    scene: Layer,
    /// How many scenes were composed: a test reads it to tell a frame
    /// that repainted the panel from one that only presented it again.
    #[cfg(test)]
    composes: usize,
}

impl Stage {
    /// A stage for a panel with `style` under it. Nothing is allocated
    /// until the first frame.
    pub fn new(style: ShadowStyle) -> Self {
        Self {
            style,
            shadow: None,
            built: None,
            panel: Layer::with_limit(LAYER_PIXELS),
            scene: Layer::with_limit(LAYER_PIXELS),
            #[cfg(test)]
            composes: 0,
        }
    }

    /// How many scenes were composed so far.
    #[cfg(test)]
    pub fn composes(&self) -> usize {
        self.composes
    }

    /// Paints the scene: `paint` draws the panel into a `panel` sized
    /// pixel slice (rows packed, the shape the panel painters take), then
    /// the shadow goes under it. `false` when a layer cannot be made (a
    /// zero or absurd size): nothing is drawn.
    pub fn compose(&mut self, at: &Scene, paint: impl FnOnce(&mut [u8], (u32, u32))) -> bool {
        if at.panel.0 == 0 || at.panel.1 == 0 || !self.scene.begin(at.surface.0, at.surface.1) {
            return false;
        }
        #[cfg(test)]
        {
            self.composes += 1;
        }
        // No room around the panel (no shadow, or a surface held to the
        // panel's size): it paints straight into the scene.
        if at.origin == (0, 0) && at.panel == at.surface {
            paint(self.scene.pixels_mut(), at.panel);
            return true;
        }
        if !self.panel.begin(at.panel.0, at.panel.1) {
            return false;
        }
        paint(self.panel.pixels_mut(), at.panel);
        self.prepare(at);
        if let Some(mut canvas) = self.scene.canvas() {
            if let Some(shadow) = &self.shadow {
                let color = match self.style {
                    ShadowStyle::Glow => at.tint,
                    _ => Color { r: 0, g: 0, b: 0 },
                };
                let card = Rect {
                    x: at.origin.0,
                    y: at.origin.1,
                    width: at.panel.0,
                    height: at.panel.1,
                };
                shadow.paint(&mut canvas, card, at.radius, color);
            }
            self.panel.composite(
                &mut canvas,
                i32::try_from(at.origin.0).unwrap_or(i32::MAX),
                i32::try_from(at.origin.1).unwrap_or(i32::MAX),
                1.0,
                255,
            );
        }
        true
    }

    /// Builds the shadow's tables when the scale, the radius or the
    /// panel's size changed since they were built.
    fn prepare(&mut self, at: &Scene) {
        let Some(spec) = self.style.spec() else {
            return;
        };
        let wanted = Built {
            factor: at.factor,
            panel: at.panel,
            radius: at.radius,
        };
        if self.built == Some(wanted) && self.shadow.is_some() {
            return;
        }
        let mut shadow = Shadow::new(scaled(spec, at.factor));
        shadow.prepare(at.radius, [at.panel]);
        self.shadow = Some(shadow);
        self.built = Some(wanted);
    }

    /// Writes the scene into `dst` (the shm buffer, `dims` pixels) through
    /// `pose`: scaled about the center and faded. A buffer is cleared
    /// first, so what the pool left in it never shows. At rest and the
    /// scene's own size it is one copy.
    pub fn present(&self, dst: &mut [u8], dims: (u32, u32), pose: Pose) {
        let Some(len) = (dims.0 as usize)
            .checked_mul(dims.1 as usize)
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            return;
        };
        let Some(dst) = dst.get_mut(..len) else {
            return;
        };
        if pose.is_rest() && self.scene.width() == dims.0 && self.scene.height() == dims.1 {
            dst.copy_from_slice(self.scene.pixels());
            return;
        }
        dst.fill(0);
        let Some(mut canvas) = Canvas::new(dst, dims.0, dims.1) else {
            return;
        };
        self.scene
            .composite(&mut canvas, 0, 0, pose.scale, pose.alpha);
    }
}

#[cfg(test)]
mod tests;
