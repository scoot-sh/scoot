//! Every output this compositor drives, and the stable [`OutputId`] the core
//! knows each one by.
//!
//! Not to be confused with `crate::output`, which is stdout/stderr for `scoot
//! msg`.
//!
//! # Why the collection lives here and not in [`scoot_core`]
//!
//! The core already models the collection: [`World`](scoot_core::World) holds
//! a `Vec` of outputs keyed by [`OutputId`], learns about them through
//! `OutputAdded`/`OutputChanged`/`OutputRemoved`, gives each one its own
//! workspaces and usable area, and stamps every `Placement` with the output it
//! belongs to. What was single here was the *binding*: one
//! `Option<smithay::output::Output>` on [`State`], and one `const OUTPUT_ID`
//! standing in for a lookup.
//!
//! A binding from a core id to a `wl_output`-backed [`Output`] is Wayland, so
//! it belongs on this side of the platform line -- a macOS Accessibility
//! adapter would keep its own id-to-`NSScreen` map against the same core.
//!
//! # What the shape commits to: one scrolling strip per output
//!
//! The core's tree gives each output its own `Vec<Workspace>`, and each
//! workspace its own columns and its own `view_x`. This collection is the
//! compositor-side half of that, and it means the same three things niri's
//! model does:
//!
//! - a window is on exactly one output at a time, never spanning two;
//! - moving a column to another output is a move within the tree, not a
//!   scroll -- there is no strip that runs across an output boundary;
//! - one output's reserved edges (a bar's exclusive zone) shrink that
//!   output's usable area and no other's.
//!
//! # What is *not* per-output yet
//!
//! This is the foundation, not the whole of multi-output (see
//! `docs/backlog/core/multi-output.md`). Every output has a render target of
//! its own (milestone 19, phase A); the protocol sites that still assume one
//! output go through [`Outputs::primary`] rather than a
//! const, so `grep primary()` enumerates them for the item that makes them
//! per-output.
//!
//! [`State`]: super::State

use scoot_core::OutputId;
use smithay::output::Output;

#[cfg(test)]
mod per_output;
#[cfg(test)]
mod tests;

/// The id of the first output created. Outputs are numbered from one so that
/// a single-output session reports the same `OutputId(1)` over IPC and to the
/// core that it always has.
const FIRST_ID: OutputId = OutputId(1);

/// One output and the id the core knows it by.
struct Entry {
    id: OutputId,
    output: Output,
}

/// The compositor's outputs, in the order they were created.
///
/// A `Vec`, not a `HashMap`: a session has a handful of outputs at most, so a
/// linear scan beats hashing on every measure that matters here, and the
/// creation order is itself meaningful ([`Outputs::primary`] is the first).
/// Every accessor allocates nothing.
pub(crate) struct Outputs {
    entries: Vec<Entry>,
    /// The id the next [`Outputs::add`] hands out. Monotonic and never
    /// reused, so an id stays stable for the life of the process even if this
    /// ever grows a removal path.
    next: OutputId,
}

impl Default for Outputs {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            next: FIRST_ID,
        }
    }
}

impl Outputs {
    /// Registers `output` and hands back the id the core will know it by.
    ///
    /// The count is bounded at startup (`--outputs`, see `cli.rs`) and
    /// nothing adds one later, so the counter cannot realistically be driven
    /// anywhere near wrapping. It saturates rather than wraps all the same --
    /// but be clear about what that buys, because an earlier version of this
    /// comment had it backwards: *both* behaviours repeat an id at the
    /// boundary. Saturation repeats the last one, wrapping repeats from the
    /// start. It is preferred only because a stuck maximum is the more
    /// obvious symptom to debug, not because it avoids the collision.
    pub(crate) fn add(&mut self, output: Output) -> OutputId {
        let id = self.next;
        self.next = OutputId(self.next.0.saturating_add(1));
        self.entries.push(Entry { id, output });
        id
    }

    /// The output every site that still assumes a single output acts on.
    ///
    /// The render loop, screenshots, screen capture and gamma controls no
    /// longer go through here -- each of those resolves its *own* output (see
    /// `headless.rs`'s `render`, `screenshot.rs`'s output check,
    /// `screencopy.rs`'s `capture_constraints` and `gamma_control.rs`'s
    /// `get_gamma_control`), because answering any of them from another
    /// output's framebuffer would hand a client a picture of one screen
    /// labelled as another. What remains here are the later phases' sites,
    /// and they do not all change the same way:
    ///
    /// - `layer_shell.rs`'s `refresh_layer_zone` needs a zone *per* output
    ///   rather than one (frame callbacks and dead-surface cleanup already
    ///   walk every output's map -- see `headless.rs`'s `render`);
    /// - `layer_hit` needs the output under the pointer;
    /// - `layer_keyboard_focus` has to consider more than one `LayerMap`;
    /// - `session_lock.rs`'s `new_surface` fallback, `configure_all`,
    ///   confirmation (`locked` must wait for *every* output's blanked frame
    ///   -- the security-relevant one) and the locked render path;
    /// - `ext_workspace.rs` needs a group per output, and
    ///   `foreign_toplevel_management.rs` an `output_enter` per output a
    ///   window is on;
    /// - `output_management.rs` needs a head per output;
    /// - `input.rs`'s pointer clamp needs the union of the outputs, or the
    ///   one the pointer is on.
    ///
    /// The two sites that already resolve an output *per surface* rather than
    /// through here are `layer_shell.rs`'s `new_layer_surface` (which honours
    /// the client's requested `wl_output`) and [`State::output_of_layer`],
    /// which finds the map a layer surface is actually in.
    ///
    /// [`State::output_of_layer`]: super::State::output_of_layer
    pub(crate) fn primary(&self) -> Option<&Output> {
        self.entries.first().map(|entry| &entry.output)
    }

    /// [`Outputs::primary`]'s core id -- what an event about the primary
    /// output is filed under.
    pub(crate) fn primary_id(&self) -> Option<OutputId> {
        self.entries.first().map(|entry| entry.id)
    }

    /// Both halves of the primary output at once, for the callers that need
    /// the [`Output`] *and* the id and must not read them through two
    /// separate `Option`s (one of which would then need an `expect`).
    pub(crate) fn primary_entry(&self) -> Option<(OutputId, &Output)> {
        self.entries.first().map(|entry| (entry.id, &entry.output))
    }

    /// The most recently added output -- where `headless::add_output` measures
    /// the next one's position from.
    pub(crate) fn last(&self) -> Option<&Output> {
        self.entries.last().map(|entry| &entry.output)
    }

    /// The output `id` names, if this compositor has one.
    pub(crate) fn get(&self, id: OutputId) -> Option<&Output> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| &entry.output)
    }

    /// The id `output` was registered under, if this compositor has it.
    ///
    /// The reverse of [`Outputs::get`], for the paths that arrive holding a
    /// Smithay [`Output`] -- a capture source's upgraded weak handle, a
    /// gamma control's `wl_output` -- and need the id the core, the render
    /// targets and the IPC layer know it by. Identity by `Output` equality,
    /// the same comparison the capture and gamma guards already used
    /// against [`Outputs::primary`].
    pub(crate) fn id_of(&self, output: &Output) -> Option<OutputId> {
        self.entries
            .iter()
            .find(|entry| entry.output == *output)
            .map(|entry| entry.id)
    }

    /// Every output, in creation order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Output> + '_ {
        self.entries.iter().map(|entry| &entry.output)
    }

    /// How many outputs this compositor has -- what bounds the render loop's
    /// index walk (see `State::render`, which cannot hold the borrow an
    /// iterator would keep while drawing).
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The `index`-th output and its id, cloned, or `None` past the end.
    ///
    /// Cloned rather than borrowed: the render loop draws with `&mut State`,
    /// which no borrow of this collection can outlive. An [`Output`] clone
    /// is an `Arc` bump, not a copy of anything drawn.
    pub(crate) fn at(&self, index: usize) -> Option<(OutputId, Output)> {
        self.entries
            .get(index)
            .map(|entry| (entry.id, entry.output.clone()))
    }

    /// Whether no output has been created yet -- true only before
    /// `headless::init_named` has run.
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
