//! What each output is meant to show: the choice for every output, and the
//! choices made for single outputs by name.
//!
//! - `scootbg set X` (every output) replaces every choice: the named ones
//!   are dropped, so each output, present or plugged in later, shows X.
//! - `scootbg set X --output NAME` is a choice for outputs with that
//!   connector name. It is kept by name, so the monitor shows it again when
//!   it is unplugged and plugged back in.
//! - `clear` is the same with nothing: back to the compositor's own
//!   background. A cleared name stays in the table as "nothing", so it
//!   shows nothing even while every other output has a color.
//!
//! **The newest request wins, whatever order they land in.** Every choice
//! carries its request's generation (`crate::waiters`). A color lands at
//! once, but an image lands only once decoded, maybe after newer requests
//! did; an older one then changes only what nothing newer has chosen
//! ([`Choices::set`]).
//!
//! A named choice is only ever requested for an output that is present
//! (the daemon refuses unknown names), or restored from the state file
//! (`crate::state`, at most `format::MAX_OUTPUTS` names, connected or
//! not), so the table holds at most one entry per connector name the
//! compositor has shown or the file names: bounded by the hardware and the
//! file, not by requests.
//!
//! ## One wallpaper per workspace
//!
//! A workspace mapping is the wallpaper for one workspace (`set
//! --workspace NAME`, through `ext-workspace-v1`, so it works on any
//! compositor with that protocol, not only scoot): on the outputs named
//! `output` (`None`: every output) while that workspace is active there.
//! Workspaces are keyed by name, the string the compositor announces (on
//! scoot the 1-based position: "1", "2", ...; see `crate::workspaces` for
//! what that means across renumbering). The daemon binds the workspace
//! manager only while a mapping exists, so never using this costs nothing:
//! no extra global, no extra wakeups.
//!
//! Lookups follow one timeline: among the base choice and every mapping
//! that applies, the newest wins. A `set` newer than a `set --workspace`
//! covers it until a still newer `set --workspace`; mappings are never
//! dropped by a `set`, only superseded, and a `clear --workspace` removes
//! one outright.

use std::sync::Arc;

use crate::transition::Spec;
use crate::wallpaper::{Image, Wallpaper};

#[cfg(test)]
mod tests;

/// What one output should show: `None` is nothing, the compositor's own
/// background.
pub type Choice = Option<Wallpaper>;

/// What one workspace should show: `None` is no mapping, the output's own
/// wallpaper (whatever `set` chose for it).
pub type WorkspaceChoice = Option<Wallpaper>;

/// One workspace mapping: the wallpaper for `workspace` on the outputs
/// named `output` (`None`: every output) while that workspace is active
/// there. See the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceEntry {
    pub output: Option<String>,
    pub workspace: String,
    pub choice: WorkspaceChoice,
    pub made: u64,
    pub transition: Spec,
}

/// The most workspace mappings kept: `set --workspace` past this is
/// refused, so a client cannot make the daemon hold workspaces without
/// bound. 64 workspaces is far past any compositor's real list (scoot
/// numbers them per output, trailing-empty last).
pub const MAX_WORKSPACES: usize = 64;

/// The longest workspace name kept, in bytes: past this a `set
/// --workspace` is refused. Compositor names are short ("1", "2", "web");
/// the bound is only against a client filling memory through long names.
pub const MAX_WORKSPACE_NAME: usize = 256;

#[derive(Debug, Default)]
pub struct Choices {
    /// For every output, and its generation (0: none made yet).
    all: (Choice, u64),
    named: Vec<(String, Choice, u64)>,
    /// One entry per (output, workspace) mapped, newest wins per key (see
    /// [`Choices::set_workspace`]). A cleared mapping stays as a `None`
    /// choice at its generation, so an older image `set --workspace`
    /// landing after cannot bring it back; lookups treat it as unmapped.
    workspaces: Vec<WorkspaceEntry>,
}

impl Choices {
    /// Records `choice`, from the request of `generation`, for every
    /// output (`output` is `None`) or for the outputs named `output`, as
    /// far as nothing newer has chosen already. Returns whether it was
    /// recorded anywhere.
    pub fn set(&mut self, output: Option<&str>, choice: Choice, generation: u64) -> bool {
        if self.supersedes(output, generation) {
            return false;
        }
        match output {
            None => {
                self.all = (choice, generation);
                // Choices made for single outputs *after* this request
                // stay: they are newer.
                self.named.retain(|(_, _, made)| *made > generation);
            }
            Some(name) => match self.named.iter_mut().find(|(n, ..)| n == name) {
                Some((_, slot, made)) => {
                    *slot = choice;
                    *made = generation;
                }
                None => self.named.push((name.to_owned(), choice, generation)),
            },
        }
        true
    }

    /// Whether a request of `generation` for `output` (every output when
    /// `None`) would change nothing, because newer choices cover all it
    /// asks for: a newer every-output choice, or, for one output, a newer
    /// choice for that name.
    pub fn supersedes(&self, output: Option<&str>, generation: u64) -> bool {
        if self.all.1 > generation {
            return true;
        }
        output.is_some_and(|name| {
            self.named
                .iter()
                .any(|(n, _, made)| n == name && *made > generation)
        })
    }

    /// What an output named `name` (or unnamed) should show, given the
    /// workspace active there now (`active`, from `ext-workspace-v1`):
    /// the newest among the base choice for it and every workspace mapping
    /// that applies (its output, or every output, with that workspace).
    /// A newer `set` wins over an older `set --workspace`, and a newer
    /// `set --workspace` over an older `set`: one timeline.
    pub fn for_output(&self, name: Option<&str>, active: Option<&str>) -> Option<&Wallpaper> {
        self.effective(name, active).0
    }

    /// The transition a switch to what `name` shows now animates through:
    /// the winning workspace mapping's, or none (at once) for the base
    /// wallpaper.
    pub fn transition_for(&self, name: Option<&str>, active: Option<&str>) -> Spec {
        self.effective(name, active).2
    }

    /// The effective choice for `name` with `active` on it: what shows,
    /// when it was made, and through what transition it arrives. The one
    /// timeline both lookups above share, so they can never disagree about
    /// which entry wins.
    fn effective(
        &self,
        name: Option<&str>,
        active: Option<&str>,
    ) -> (Option<&Wallpaper>, u64, Spec) {
        let (mut choice, mut made) =
            match name.and_then(|name| self.named.iter().find(|(n, ..)| n == name)) {
                Some((_, choice, made)) => (choice.as_ref(), *made),
                None => (self.all.0.as_ref(), self.all.1),
            };
        let mut spec = Spec::none();
        if let Some(active) = active {
            for entry in &self.workspaces {
                let for_this_output = match &entry.output {
                    None => true,
                    Some(output) => Some(output.as_str()) == name,
                };
                // A cleared mapping (`None`) is ordering only: it never
                // wins the lookup (the base shows), but its generation
                // still refuses older landings for its key.
                if for_this_output
                    && entry.choice.is_some()
                    && entry.workspace == active
                    && entry.made > made
                {
                    choice = entry.choice.as_ref();
                    made = entry.made;
                    spec = entry.transition;
                }
            }
        }
        (choice, made, spec)
    }

    /// The choice for every output, if one was ever made (a `clear` of
    /// every output is one: nothing).
    pub fn every(&self) -> Option<&Choice> {
        (self.all.1 > 0).then_some(&self.all.0)
    }

    /// The choices for single outputs, by name, with the generation each
    /// was made at (larger: more recent), in the order the names were
    /// first chosen.
    pub fn named(&self) -> impl Iterator<Item = (&str, &Choice, u64)> {
        self.named
            .iter()
            .map(|(name, choice, made)| (name.as_str(), choice, *made))
    }

    /// The choice made exactly for `output` (every output when `None`), not
    /// what an output of that name falls back to: `None` when none was made
    /// there.
    pub fn exact(&self, output: Option<&str>) -> Option<&Choice> {
        match output {
            None => self.every(),
            Some(name) => self
                .named
                .iter()
                .find(|(n, ..)| n == name)
                .map(|(_, choice, _)| choice),
        }
    }

    /// Records the wallpaper for `workspace` on `output` (`None`: every
    /// output), from the request of `generation`, arriving through
    /// `transition` when its workspace turns active. Newest wins per
    /// (output, workspace) key: an older request landing after a newer one
    /// changes nothing and reports `false`. A cleared mapping (`None`) is
    /// kept at its generation rather than removed, so an older image
    /// request landing after cannot bring it back; lookups treat it as
    /// unmapped. At most [`MAX_WORKSPACES`] live mappings are kept: a new
    /// key past that reports `false` (updating a key held already always
    /// works; cleared keys do not count).
    pub fn set_workspace(
        &mut self,
        output: Option<&str>,
        workspace: &str,
        choice: WorkspaceChoice,
        transition: Spec,
        generation: u64,
    ) -> bool {
        match self
            .workspaces
            .iter_mut()
            .find(|entry| entry.output.as_deref() == output && entry.workspace == workspace)
        {
            Some(entry) => {
                if entry.made > generation {
                    return false;
                }
                entry.choice = choice;
                entry.made = generation;
                entry.transition = transition;
                true
            }
            None => {
                let live = self
                    .workspaces
                    .iter()
                    .filter(|entry| entry.choice.is_some())
                    .count();
                if live >= MAX_WORKSPACES {
                    return false;
                }
                self.workspaces.push(WorkspaceEntry {
                    output: output.map(str::to_owned),
                    workspace: workspace.to_owned(),
                    choice,
                    made: generation,
                    transition,
                });
                true
            }
        }
    }

    /// Whether `serial` is a workspace image mapped for the output named
    /// `name` (its own, or every output's): for keeping renders the worker
    /// is asked for, though nothing shows them yet.
    pub fn workspace_image_for(&self, name: Option<&str>, serial: u64) -> bool {
        self.workspaces.iter().any(|entry| {
            let for_this_output = match &entry.output {
                None => true,
                Some(output) => Some(output.as_str()) == name,
            };
            for_this_output
                && matches!(&entry.choice, Some(Wallpaper::Image(image)) if image.serial == serial)
        })
    }

    /// Every workspace image mapped for the output named `name`, newest
    /// first, deduplicated: what an output reconfigured to a new size
    /// pre-renders so its switches stay instant.
    pub fn workspace_images_for(&self, name: Option<&str>) -> Vec<Arc<Image>> {
        let mut images: Vec<(u64, Arc<Image>)> = Vec::new();
        for entry in &self.workspaces {
            let for_this_output = match &entry.output {
                None => true,
                Some(output) => Some(output.as_str()) == name,
            };
            if !for_this_output {
                continue;
            }
            if let Some(Wallpaper::Image(image)) = &entry.choice {
                if !images.iter().any(|(serial, _)| *serial == image.serial) {
                    images.push((image.serial, Arc::clone(image)));
                }
            }
        }
        images.sort_by_key(|(serial, _)| std::cmp::Reverse(*serial));
        images.into_iter().map(|(_, image)| image).collect()
    }

    /// The serial of the image mapped for (`output`, `workspace`), if the
    /// mapping holds one: for dropping its stashed pixels once replaced.
    pub fn workspace_serial(&self, output: Option<&str>, workspace: &str) -> Option<u64> {
        self.workspaces
            .iter()
            .find(|entry| entry.output.as_deref() == output && entry.workspace == workspace)
            .and_then(|entry| entry.choice.as_ref())
            .and_then(Wallpaper::image)
            .map(|image| image.serial)
    }

    /// Whether any workspace mapping with a wallpaper stands (cleared ones
    /// do not count): while none does the daemon binds no workspace
    /// manager, at zero cost.
    pub fn has_workspace_mappings(&self) -> bool {
        self.workspaces.iter().any(|entry| entry.choice.is_some())
    }

    /// The live mappings, in the order they were first made: the output
    /// each is for (`None`: every output), the workspace, what it shows
    /// (`None`: cleared, kept only for ordering against older requests),
    /// when it was made, and through what transition it arrives.
    pub fn workspaces(
        &self,
    ) -> impl Iterator<Item = (Option<&str>, &str, &WorkspaceChoice, u64, Spec)> {
        self.workspaces.iter().map(|entry| {
            (
                entry.output.as_deref(),
                entry.workspace.as_str(),
                &entry.choice,
                entry.made,
                entry.transition,
            )
        })
    }

    /// Replaces the value of the choice made exactly for `output`, keeping
    /// its generation, so it is as new as it was and no newer: for putting
    /// back an image `apply-config` chose but could not show then
    /// (`daemon::config`). Returns whether there was such a choice.
    pub fn fill(&mut self, output: Option<&str>, choice: Choice) -> bool {
        match output {
            None if self.all.1 > 0 => {
                self.all.0 = choice;
                true
            }
            None => false,
            Some(name) => match self.named.iter_mut().find(|(n, ..)| n == name) {
                Some((_, slot, _)) => {
                    *slot = choice;
                    true
                }
                None => false,
            },
        }
    }

    /// Drops the choice for `name`, as if none had been made: for the
    /// saved table only (`crate::state`), when the file has no room for it.
    pub fn forget(&mut self, name: &str) {
        self.named.retain(|(n, ..)| n != name);
    }

    /// Drops the mapping for (`output`, `workspace`), as if none had been
    /// made: for the saved table only (`crate::state`), when the file has
    /// no room for it.
    pub fn forget_workspace(&mut self, output: Option<&str>, workspace: &str) {
        self.workspaces
            .retain(|entry| entry.output.as_deref() != output || entry.workspace != workspace);
    }

    /// Drops every workspace mapping, as if none had been made: adopting a
    /// profile whose file says nothing of them (`daemon::config`).
    pub fn clear_workspaces(&mut self) {
        self.workspaces.clear();
    }

    #[cfg(test)]
    pub fn named_len(&self) -> usize {
        self.named.len()
    }
}
