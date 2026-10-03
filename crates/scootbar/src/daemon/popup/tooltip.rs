//! A module's tooltip: the same `xdg_popup` as a click popup, made after the
//! pointer has rested on a module with a tooltip, and gone when it leaves.
//! What is different, and why:
//!
//! - **No grab, no keyboard, no input region.** A tooltip never takes the
//!   keyboard (`get_keyboard` is not called, `grab` is not asked), and its
//!   surface has an empty input region, so the pointer never enters it and
//!   a click under it lands on the bar. It cannot steal a click, and it
//!   cannot cause its own `leave` (a tooltip flipped over the module the
//!   pointer is on would otherwise close itself and reopen).
//! - **Its own slot** ([`Popups::tip`]). The click popup's slot keeps its
//!   one meaning (a popup that may hold a grab), so a press that finds a
//!   popup open still dismisses it and is spent, while a press that finds
//!   only a tooltip closes it and *acts*. One popup at a time still holds: a
//!   click popup closes the tooltip when it opens, and no tooltip shows
//!   while one is open.
//! - **Drawn from the module's view.** The text is the `tooltip` the view
//!   already carries (`Module::view`), wrapped ([`Content::wrap_tooltip`]).
//!   A module is asked for nothing new, and a module without one has none.
//!
//! ## When it shows and goes
//!
//! The deadline is `crate::popup::Hover`'s, and it is the loop's `poll`
//! timeout while one is pending ([`State::tooltip_timeout`]): no timer file
//! descriptor, no wakeup with nothing hovered. **Dismissed by**: the pointer
//! leaving the module or the bar (the loop sees the bar's `leave`), any
//! press, any scroll, a click popup opening, the module's text going empty,
//! the compositor's `popup_done` (a session lock), the output, the bar or
//! the module going, a scale change, a reload. After a press, a scroll or
//! the compositor's `popup_done` it stays dismissed until the pointer has
//! left the module.
//!
//! ## While it is shown
//!
//! Its text changing (a clock's tooltip ticking) redraws it in place when
//! the size it was opened at holds it (a shorter text keeps the size: no
//! flicker per tick), else it is made again at once at
//! the new size (a popup is never resized after its configure). The module
//! moving along the bar (a neighbor's text grew) makes it again at the new
//! anchor.

use std::time::{Duration, Instant};

use wayland_client::QueueHandle;

use super::{Flavor, Popups, anchor_span, draw_open};
use crate::daemon::wayland::State;
use crate::density::Scale;
use crate::popup::{Key, Step};
use crate::render;

impl State {
    /// How long the loop may sleep for a tooltip that is due: `Some` only
    /// while the pointer rests on a module that has one and it has not shown.
    pub fn tooltip_timeout(&self, now: &mut Option<Instant>) -> Option<Duration> {
        self.popup
            .hover
            .wait(&mut || *now.get_or_insert_with(Instant::now))
    }

    /// A press or a scroll: the tooltip goes, and stays gone until the
    /// pointer leaves the module.
    pub fn dismiss_tooltip(&mut self) {
        if !self.popup.hover.enabled() {
            return;
        }
        let under = self.hovered_tooltip().map(|(key, _)| key);
        self.popup.hover.dismiss(under);
        self.popup.close_tip();
    }

    /// Once a turn, after the bars are drawn (so what a module's view says
    /// is what the scene says): shows, keeps up and removes the tooltip.
    /// Nothing at all (three checks) while nothing is hovered and none is
    /// shown or due.
    pub fn pump_tooltip(&mut self, qh: &QueueHandle<State>, now: &mut Option<Instant>) {
        if !self.popup.hover.enabled()
            || (self.input.focus().is_none()
                && self.popup.tip.is_none()
                && self.popup.hover.is_idle())
        {
            return;
        }
        // A tooltip the compositor took back (`popup_done`), or that closed
        // with its bar: a dismissal, not an absence to open it again from.
        if self.popup.hover.is_shown() && self.popup.tip.is_none() {
            self.popup.hover.gone();
        }
        let hovered = self.hovered_tooltip();
        let blocked = self.popup.is_open();
        let step = self
            .popup
            .hover
            .update(hovered.map(|(key, _)| key), blocked, &mut || {
                *now.get_or_insert_with(Instant::now)
            });
        match step {
            Step::Nothing => {}
            Step::Hide => self.popup.close_tip(),
            Step::Show(key) => {
                let member = hovered.filter(|(hovered, _)| *hovered == key).map(|h| h.1);
                // Not a user's request: a failure is a tooltip that does not
                // show, said nowhere (it would be a line per hover).
                if !member.is_some_and(|member| self.open_tooltip(qh, key, member)) {
                    self.popup.hover.dismiss(None);
                }
            }
        }
        self.keep_tooltip(qh);
    }

    /// The module under the pointer, if it shows a tooltip: its key and its
    /// place on that output's bar.
    fn hovered_tooltip(&self) -> Option<(Key, usize)> {
        let target = self.target()?;
        let entry = self
            .outputs
            .iter()
            .find(|entry| entry.output.id() == target.output)?;
        let view = entry.objects.scene.view(target.member)?;
        if view.tooltip().is_empty() {
            return None;
        }
        let module = entry.objects.scene.module(target.member)?;
        Some((
            Key {
                output: target.output,
                module,
            },
            target.member,
        ))
    }

    /// Makes the tooltip; whether it did.
    fn open_tooltip(&mut self, qh: &QueueHandle<State>, key: Key, member: usize) -> bool {
        self.popup.close_tip();
        match self.build_popup(qh, key.output, member, Flavor::Tooltip) {
            Ok(tip) => {
                self.popup.tip = Some(tip);
                true
            }
            Err(_) => false,
        }
    }

    /// While shown: closes it when what it hangs off is gone, follows its
    /// module, redraws it when its text changed or the compositor sized it.
    fn keep_tooltip(&mut self, qh: &QueueHandle<State>) {
        let Some(tip) = self.popup.tip.as_mut() else {
            return;
        };
        let (output, module) = (tip.output, tip.module);
        let Some(entry) = self.outputs.get_mut(output) else {
            self.popup.close_tip();
            return;
        };
        let Some(layer) = entry.objects.layer.as_ref() else {
            self.popup.close_tip();
            return;
        };
        let drawn = entry.output.scale();
        let scale = if layer.viewport.is_some() {
            drawn
        } else {
            Scale::Integer(drawn.integer())
        };
        let bar_width = entry
            .output
            .surface_size(&entry.objects.bar)
            .map_or(0, |size| size.width);
        let member = Popups::member_of(&entry.objects.scene, module);
        let anchor = member
            .and_then(|member| entry.objects.scene.spans().get(member).copied())
            .map(|span| anchor_span(scale, span, bar_width));
        let (Some(member), Some(anchor)) = (member, anchor) else {
            self.popup.close_tip();
            return;
        };
        let (Some(placed), Some(text)) =
            (self.content.modules.get(module), self.content.text.as_mut())
        else {
            self.popup.close_tip();
            return;
        };
        // Moved (or resized) on the bar, or drawn at another scale: made
        // again where it belongs, with no new delay.
        let mut again = scale != tip.scale || anchor != tip.anchor;
        if !again && placed.revision != tip.revision {
            tip.revision = placed.revision;
            let spare = &mut self.popup.spare;
            let style = &self.content.style;
            let em = tip.em;
            let pad = render::device(style.padding, scale);
            let frame = render::device(1, scale).max(1);
            let room = render::device(bar_width, scale)
                .saturating_sub(pad.saturating_add(frame).saturating_mul(2));
            let wide = ((em * crate::popup::TIP_MAX_EM) as u32).min(room).max(1);
            let line = entry
                .objects
                .scene
                .view(member)
                .map_or("", |view| view.tooltip());
            spare.wrap_tooltip(line, wide, |line| text.measure(None, line, em));
            if spare.is_empty() {
                self.popup.close_tip();
                return;
            }
            if *spare != tip.content {
                // Into the tooltip's own layout (its rows are reused); if the
                // size changed it is made again below, so nothing is lost.
                tip.layout.compute_tooltip(spare, text, em, pad, frame);
                let fits = scale
                    .buffer(crate::outputs::Size {
                        width: scale.logical_ceil(tip.layout.width),
                        height: scale.logical_ceil(tip.layout.height),
                    })
                    .is_some_and(|dims| dims.0 <= tip.dims.0 && dims.1 <= tip.dims.1);
                if fits {
                    std::mem::swap(spare, &mut tip.content);
                    tip.layout.width = tip.dims.0;
                    tip.layout.height = tip.dims.1;
                    tip.dirty = true;
                } else {
                    again = true;
                }
            }
        }
        if again {
            let key = Key { output, module };
            self.popup.close_tip();
            if !self.open_tooltip(qh, key, member) {
                self.popup.hover.dismiss(None);
            }
            return;
        }
        if let Some(tip) = self.popup.tip.as_mut() {
            if !draw_open(tip, &self.globals, qh, text, &self.content.style.theme) {
                self.popup.close_tip();
            }
        }
    }
}
