//! The agent interface: what `query`, `layout` and `invoke` answer, written
//! from the same state the screen is drawn from, so what an agent reads is
//! what a screenshot shows.
//!
//! - **`query`** lists each placed module's view on every output that shows
//!   it ([`views`]): id, section, output, text, class, icon, tooltip and a
//!   `value` where the module has one. The text is asked of the module as
//!   the bar asks it, into one reused [`View`].
//! - **`layout`** lists each output's bar rectangle and each module's
//!   rectangle in the compositor's global logical pixels, **as last drawn**:
//!   the spans are the scene's own (`render::Scene`, the layout the last
//!   committed frame used), converted with the scale that frame was drawn at,
//!   rounded outward so a point inside a rectangle is on the module. A hidden
//!   or not-yet-configured bar has no rectangle and no modules.
//! - **`invoke`** runs an action exactly as a click or a scroll would: the
//!   same [`crate::action::perform`] a pointer press ends in, so what an
//!   agent does and what a user does cannot diverge.
//!
//! Replies are bounded ([`MAX_REPLY`]): the modules (at most 32) times the
//! outputs, each text and tooltip at most 256 bytes.

use std::borrow::Cow;

#[cfg(test)]
mod tests;

use super::input::Launch;
use super::surfaces::Objects;
use super::wayland::State;
use crate::action::{Action, ModuleAction, Trigger, perform};
use crate::bar::{Bar, Edge};
use crate::control::protocol::{
    ModuleView, OutputLayout, PlacedRect, Point, Rect, Reply, write_reply,
};
use crate::density::Scale;
use crate::modules::{ArgKind, OutputView, Placed, View};
use crate::outputs::{Outputs, Size};

/// The longest reply written, in bytes. A reply that would pass it is
/// replaced by an error saying so: past it a client would be reading more
/// than any real bar holds.
pub const MAX_REPLY: usize = 512 * 1024;

/// Calls `each` with every placed module's view on every output that shows
/// it, in start order and then output order. `only` is a module's index in
/// `placed`, to list that one alone. `scratch` is the one view they are all
/// asked into.
pub(super) fn views(
    placed: &[Placed],
    outputs: &Outputs<Objects>,
    scratch: &mut View,
    only: Option<usize>,
    mut each: impl FnMut(ModuleView<'_>) -> bool,
) {
    for (index, module) in placed.iter().enumerate() {
        if only.is_some_and(|only| only != index) {
            continue;
        }
        for entry in outputs.iter() {
            // Only the outputs that show it, in the section they place it.
            let Some(section) = entry.objects.scene.section_of(index) else {
                continue;
            };
            let name = entry.output.info().name.as_deref();
            let output = OutputView { name };
            scratch.clear();
            module.module.view(&output, scratch);
            let view = ModuleView {
                id: module.id,
                section: section.name(),
                output: name,
                text: scratch.text(),
                class: scratch.class().name(),
                icon: scratch.icon(),
                tooltip: scratch.tooltip(),
                value: module.module.value(&output),
            };
            if !each(view) {
                return;
            }
        }
    }
}

/// The `query` reply, for one module or all. `Err` is the error reply's
/// message (a module that is not placed).
pub(super) fn write_query(
    out: &mut Vec<u8>,
    placed: &[Placed],
    outputs: &Outputs<Objects>,
    scratch: &mut View,
    id: Option<&str>,
) -> Result<(), String> {
    let only = match id {
        None => None,
        Some(id) => Some(
            placed
                .iter()
                .position(|p| p.id == id)
                .ok_or_else(|| not_placed(id, placed))?,
        ),
    };
    let start = out.len();
    // Every element serializes (strings, numbers, a char), and writing
    // into a `Vec` cannot fail: `failed` is still checked rather than
    // assumed, so a half-written line never goes out.
    let mut failed = false;
    let mut first = true;
    out.extend_from_slice(br#"{"type":"modules","modules":["#);
    views(placed, outputs, scratch, only, |view| {
        if !first {
            out.push(b',');
        }
        first = false;
        if serde_json::to_writer(&mut *out, &view).is_err() {
            failed = true;
            return false;
        }
        // A reply past the bound is cut here, not at the client.
        out.len() - start <= MAX_REPLY
    });
    if failed || out.len() - start > MAX_REPLY {
        out.truncate(start);
        let message = if failed {
            "internal: reply failed to encode".to_owned()
        } else {
            format!("the reply would be longer than {MAX_REPLY} bytes")
        };
        write_reply(out, &Reply::Error { message: &message });
        return Ok(());
    }
    out.extend_from_slice(b"]}");
    out.push(b'\n');
    Ok(())
}

/// The bar's rectangle on an output at `origin` whose logical size is
/// `output`, for a surface of `surface` logical pixels: the layer-shell
/// placement of a surface anchored to one edge and both sides, which is
/// where the protocol says it goes (the margins in from the edges).
pub(super) fn bar_rect(bar: &Bar, origin: (i32, i32), output: Size, surface: Size) -> Rect {
    let [top, _, bottom, left] = bar.margins();
    let height = i64::from(surface.height);
    let y = match bar.edge {
        Edge::Top => i64::from(origin.1) + i64::from(top),
        Edge::Bottom => i64::from(origin.1) + i64::from(output.height) - i64::from(bottom) - height,
    };
    Rect {
        x: clamp_i32(i64::from(origin.0) + i64::from(left)),
        y: clamp_i32(y),
        width: surface.width,
        height: surface.height,
    }
}

/// A module's rectangle: its device-pixel span `x..x + width` on a bar at
/// `bar`, at `scale`, rounded outward (the start down, the end up).
pub(super) fn module_rect(x: u32, width: u32, scale: Scale, bar: Rect) -> Rect {
    let start = scale.logical_floor(x);
    let end = scale.logical_ceil(x.saturating_add(width)).min(bar.width);
    Rect {
        x: clamp_i32(i64::from(bar.x) + i64::from(start)),
        y: bar.y,
        width: end.saturating_sub(start),
        height: bar.height,
    }
}

fn clamp_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

/// The `layout` reply: every output's bar and the modules it shows, as last
/// drawn.
pub(super) fn write_layout(out: &mut Vec<u8>, state: &State) {
    let start = out.len();
    let placed = &state.content.modules;
    let mut layouts = Vec::new();
    for entry in state.outputs.iter() {
        let info = entry.output.info();
        let scale = entry.output.scale();
        let bar = entry
            .output
            .surface_size(&entry.objects.bar)
            .zip(info.device())
            .map(|(surface, device)| {
                bar_rect(
                    &entry.objects.bar,
                    info.origin,
                    scale.logical(device),
                    surface,
                )
            });
        let scene = &entry.objects.scene;
        let mut modules = Vec::new();
        if let Some(bar) = bar {
            for (member, span) in scene.spans().iter().enumerate() {
                if span.width == 0 {
                    continue;
                }
                let Some(module) = scene.module(member).and_then(|m| placed.get(m)) else {
                    continue;
                };
                let Some(section) = scene
                    .module(member)
                    .and_then(|index| scene.section_of(index))
                else {
                    continue;
                };
                modules.push(PlacedRect {
                    id: module.id,
                    section: section.name(),
                    rect: module_rect(span.x, span.width, scale, bar),
                });
            }
        }
        layouts.push(OutputLayout {
            output: info.name.as_deref(),
            origin: Point {
                x: info.origin.0,
                y: info.origin.1,
            },
            scale: scale.factor(),
            bar,
            modules,
        });
    }
    #[derive(serde::Serialize)]
    struct Layout<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        outputs: &'a [OutputLayout<'a>],
    }
    let reply = Layout {
        kind: "layout",
        outputs: &layouts,
    };
    if serde_json::to_writer(&mut *out, &reply).is_err() || out.len() - start > MAX_REPLY {
        out.truncate(start);
        write_reply(
            out,
            &Reply::Error {
                message: &format!("the reply would be longer than {MAX_REPLY} bytes"),
            },
        );
        return;
    }
    out.push(b'\n');
}

/// The error for a module that is not placed, naming what is.
pub(super) fn not_placed(id: &str, placed: &[Placed]) -> String {
    if placed.is_empty() {
        return format!("`{id}` is not placed: this bar shows nothing");
    }
    let mut shows = String::new();
    for module in placed {
        shows.push(' ');
        shows.push_str(module.id);
    }
    format!("`{id}` is not placed in this bar (it shows:{shows})")
}

/// The trigger `action` names (`click`, `scroll-up`, ...: a binding's key
/// without its `on-`), if it names one.
fn trigger_named(action: &str) -> Option<Trigger> {
    Trigger::ALL
        .into_iter()
        .find(|trigger| trigger.key().strip_prefix("on-") == Some(action))
}

/// Runs `action` of module `id`, as a click or a scroll would, with `arg`
/// on `output` (by default the first that shows the module).
pub(super) fn invoke(
    state: &mut State,
    id: &str,
    action: &str,
    arg: Option<i32>,
    output: Option<&str>,
) -> Result<(), String> {
    let placed = &state.content.modules;
    let index = placed
        .iter()
        .position(|p| p.id == id)
        .ok_or_else(|| not_placed(id, placed))?;
    // The outputs that show it, by name; the one asked for or the first.
    let showing: Vec<Option<&str>> = state
        .outputs
        .iter()
        .filter(|entry| entry.objects.scene.section_of(index).is_some())
        .map(|entry| entry.output.info().name.as_deref())
        .collect();
    let name: Option<String> = match output {
        Some(wanted) => {
            if !showing.contains(&Some(wanted)) {
                let mut on = String::new();
                for name in showing.iter().flatten() {
                    on.push(' ');
                    on.push_str(name);
                }
                return Err(if on.is_empty() {
                    format!("`{id}` is not shown on any output")
                } else {
                    format!(
                        "`{id}` is not shown on output `{}` (it is shown on:{on})",
                        wanted.escape_debug()
                    )
                });
            }
            Some(wanted.to_owned())
        }
        None => showing.first().and_then(|name| name.map(str::to_owned)),
    };
    let output_view = OutputView {
        name: name.as_deref(),
    };

    let Some(placed) = state.content.modules.get_mut(index) else {
        return Err(not_placed(id, &[]));
    };
    let Placed {
        id: module_id,
        module,
        bindings,
        revision,
    } = placed;
    let trigger = trigger_named(action);
    let spec_actions = crate::modules::find(module_id).map_or(&[][..], |spec| spec.actions);
    let owned;
    let (action_ref, steps): (&Action, Option<u32>) = if let Some(trigger) = trigger {
        let Some(bound) = bindings.get(trigger) else {
            return Err(format!(
                "`{id}` has no {} binding (`scootbar msg invoke {id} ACTION` runs one of its own \
                 actions instead)",
                trigger.key()
            ));
        };
        let steps = match (arg, trigger) {
            (None, Trigger::ScrollUp | Trigger::ScrollDown) => Some(1),
            (Some(n), Trigger::ScrollUp | Trigger::ScrollDown) => Some(
                u32::try_from(n)
                    .ok()
                    .filter(|n| (1..=crate::pointer::MAX_STEPS).contains(n))
                    .ok_or_else(|| {
                        format!(
                            "`{action}` takes a number of steps from 1 to {}",
                            crate::pointer::MAX_STEPS
                        )
                    })?,
            ),
            (None, _) => None,
            (Some(_), _) => return Err(format!("`{action}` takes no number")),
        };
        (bound, steps)
    } else if let Some(spec) = spec_actions.iter().find(|a| a.name == action) {
        match (spec.arg, arg) {
            (ArgKind::Required, None) => {
                return Err(format!("`{id}` `{action}` takes a whole number"));
            }
            (ArgKind::None, Some(_)) => return Err(format!("`{id}` `{action}` takes no number")),
            _ => {}
        }
        owned = Action::Module(ModuleAction {
            name: Cow::Borrowed(spec.name),
            arg,
        });
        (&owned, None)
    } else {
        let mut known = String::new();
        for spec in spec_actions {
            known.push(' ');
            known.push_str(spec.name);
        }
        for trigger in Trigger::ALL {
            known.push(' ');
            known.push_str(trigger.key().strip_prefix("on-").unwrap_or(trigger.key()));
        }
        return Err(format!(
            "`{id}` has no action `{}` (it has:{known})",
            action.escape_debug()
        ));
    };
    let mut effects = Launch {
        spawner: &mut state.spawner,
    };
    perform(
        &mut **module,
        revision,
        &output_view,
        action_ref,
        steps,
        &mut effects,
    )
    .map_err(|failure| format!("`{id}` {action}: {failure}"))
}
