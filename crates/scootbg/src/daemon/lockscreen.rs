//! Keeping the lock screen's pictures current: each output's picture is
//! the blur of the wallpaper it is meant to show now.
//!
//! **Nothing here computes a blur.** The worker made it with the pixels
//! (`daemon::worker`), and it lives with them (`canvas::Memory`), so what
//! this does each turn of the loop is compare, per output, what the
//! choices say it shows now with what its picture was made from
//! (`crate::lockscreen::Source`), and write a file only when they differ.
//! Idle, that is a handful of comparisons and no allocation, no system
//! call and no wakeup.
//!
//! **The choice, not the frame.** An output's picture follows the
//! wallpaper it is going to (`Choices::for_output`, which already follows
//! the active workspace), not the frames of a transition on the way. A
//! switch to a workspace whose pixels were stashed re-points the picture
//! at the stashed pixels' blur: no decode, no blur.
//!
//! **Until the new pixels exist** (a `set` is still decoding) the output
//! keeps the picture of the wallpaper it still shows, which is the right
//! one: that is what is on screen.
//!
//! An output with no name (a `wl_output` older than version 4 whose
//! `xdg_output` never named it) cannot be addressed by a locker and gets
//! none; one with nothing chosen (`clear`) loses its picture, so the
//! locker's own background stands.

use super::change::image_dims;
use super::wayland::State;
use crate::lockblur::solid_png;
use crate::lockscreen::Source;
use crate::print::warn;
use crate::wallpaper::Wallpaper;

/// Brings the lock screen's pictures in line with what the outputs show.
pub fn sync(state: &mut State) {
    let State {
        outputs,
        choices,
        lockscreen,
        ..
    } = state;
    if !lockscreen.enabled() {
        return;
    }
    // An output that was unplugged: its picture goes.
    lockscreen.retain(|id| outputs.iter().any(|entry| entry.output.id() == id));
    for entry in outputs.iter() {
        let output = &entry.output;
        let id = output.id();
        let Some(name) = output.info().name.as_deref() else {
            continue;
        };
        let Some(dims) = image_dims(output) else {
            // No size yet: nothing to make a picture at.
            continue;
        };
        match choices.for_output(Some(name), output.active_workspace()) {
            None => lockscreen.remove(id),
            Some(Wallpaper::Color(color)) => {
                let source = Source::Color(*color);
                if lockscreen.has(id, name, source) {
                    continue;
                }
                match solid_png(*color, dims.0, dims.1) {
                    Ok(png) => lockscreen.put(id, name, source, &png),
                    Err(error) => {
                        warn(format_args!(
                            "scootbg: no lock screen picture for {name}: {error}"
                        ));
                        lockscreen.withhold(id, name, source);
                    }
                }
            }
            Some(Wallpaper::Image(image)) => {
                let source = Source::Image(image.serial, dims);
                if lockscreen.has(id, name, source) {
                    continue;
                }
                // Not rendered here yet: the old picture stands.
                let Some(pixels) = entry.objects.canvas.image(image.serial, dims) else {
                    continue;
                };
                match pixels.memory().lock_png() {
                    Some(png) => lockscreen.put(id, name, source, png),
                    // The blur could not be made (said by the worker): the
                    // old picture is of another wallpaper, so none is
                    // better than that one.
                    None => lockscreen.withhold(id, name, source),
                }
            }
        }
    }
    lockscreen.flush();
}
