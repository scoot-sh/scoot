//! The looks module: the active look's name, and a popup listing the catalog.
//!
//! The bar text is the feedback the look-cycle binding needs: whatever
//! applied last, through this picker, `scoot look apply`, a keybinding or
//! an agent, shows here. A click opens the picker (`on-click = "popup"`,
//! and a bare click with no binding, as the network list's): one row per
//! look in catalog order, the active one selected and marked, a row click
//! running `scoot look apply` for that look. The active row carries `●`
//! before its name, or the module's glyph icon where one is set, as the
//! power menu's row glyphs do. A long catalog pages past the popup's
//! 16-widget cap ([`crate::popup::MAX_WIDGETS`]): 14 rows with `‹ prev`
//! and `next ›` rows.
//!
//! The picker uses the same machinery as `scoot look`, never a second one:
//! the rows come from [`scoot_look::catalog::discover`], and a pick stages
//! `scoot look apply NAME` for the bar to spawn, which renders, validates,
//! repoints and drives every app's look-scoped reload itself (rolling back
//! on a refusal). The popup row closes on a pick, so the swap's reload
//! finds no popup; a swap from elsewhere restyles an open picker to the new
//! style, and its next refill marks the new active look.
//!
//! Idle cost: nothing. The module holds no fd and no timer, and reads the
//! state only when asked for its view or its popup (both cold paths, never
//! per frame): the record for the name, the catalog for the rows, each read
//! bounded (capped, nonblocking, regular files only, as the look layer's
//! reads are).

use std::fmt::Write;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use rustix::event::PollFlags;

use super::{ActionSpec, ArgKind, Init, InvokeError, Module, OutputView, Sources, Update, View};
use crate::action::{Action, ModuleAction, Trigger};
use crate::icon::Icon;
use crate::modules::Input;

#[cfg(test)]
mod tests;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "looks";

/// The picker's active-row marker with no glyph icon configured: U+25CF,
/// in DejaVu, so it needs no icon font. A configured glyph icon overrides
/// it.
pub const DEFAULT_MARKER: char = '●';

/// The pad an inactive row carries where the active row has its marker and
/// gap: the bar's font is proportional (DejaVu Sans), where `●` runs about
/// three spaces wide, so marker-plus-gap reads four. Exact for the default
/// marker at any size (both scale together); a configured glyph icon is
/// near one em itself, so the same four holds within a few pixels.
const INACTIVE_PAD: &str = "    "; // Four spaces: the marker's width.

/// The actions a binding or an agent may name: `apply N` runs
/// `scoot look apply` for the picker's Nth row (what a row click does),
/// refused past the picker's rows or with no `scoot` on PATH; `page N`
/// turns the picker to page N, plus the popup itself
/// (`on-click = "popup"`). Both take a number.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "apply",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "page",
        arg: ArgKind::Required,
    },
    #[cfg(feature = "popup")]
    ActionSpec {
        name: crate::action::POPUP,
        arg: ArgKind::None,
    },
];

/// Look rows per picker page: the popup holds at most 16 widgets, less the
/// two page rows. Without the `popup` feature there is no picker to page,
/// so nothing reads it there.
#[cfg_attr(not(feature = "popup"), allow(dead_code))]
const PAGE_ROWS: usize = 14;

/// The module's options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// An icon drawn before the active look's name. None shows the name
    /// alone. A glyph icon also marks the active look's picker row,
    /// overriding the default marker.
    pub icon: Option<Icon>,
}

pub fn init(settings: &super::Settings) -> Init {
    Init::Available(Box::new(started(
        settings.looks.icon.clone(),
        crate::look::state_dir(),
    )))
}

fn started(icon: Option<Icon>, state_dir: Option<PathBuf>) -> Looks {
    Looks {
        icon,
        scoot: resolve_on_path("scoot", std::env::var_os("PATH").as_deref()),
        state_dir,
        page: 0,
        pages: 0,
        shown: Vec::new(),
        staged: None,
    }
}

/// Resolves `program` against `path` (a `PATH` value): the first executable
/// regular file by that name, as spawning it would run. `None` when
/// nothing there runs it.
fn resolve_on_path(program: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let path = path?;
    for dir in std::env::split_paths(path) {
        let candidate = dir.join(program);
        let executable = std::fs::metadata(&candidate)
            .map(|meta| meta.is_file() && (meta.permissions().mode() & 0o111) != 0)
            .unwrap_or(false);
        if executable {
            return Some(candidate);
        }
    }
    None
}

/// The module.
struct Looks {
    icon: Option<Icon>,
    /// The `scoot` binary an apply stages, resolved from PATH once at
    /// start. `None` refuses the apply with an error, so `invoke` never
    /// answers success for a spawn that cannot run.
    scoot: Option<PathBuf>,
    /// Where the active look is recorded: resolved once, read on every view
    /// and popup fill, so the bar always shows the last swap, whoever asked
    /// for it.
    state_dir: Option<PathBuf>,
    /// The picker's current page, and how many the last fill saw.
    page: usize,
    pages: usize,
    /// The names the picker showed on this page, in order: `apply N` names
    /// one of these, never a position re-read from a changed catalog.
    shown: Vec<String>,
    /// The `scoot look apply` a performed row staged, taken once by the loop
    /// through [`Module::take_action`].
    staged: Option<Action>,
}

impl Looks {
    /// The active look's name, read fresh: missing state, no record yet and
    /// a corrupt record are all no name, never an error (startup never fails
    /// over a look, and neither does this text).
    fn active(&self) -> Option<String> {
        let dir = self.state_dir.as_ref()?;
        match scoot_look::state::read_record(dir) {
            scoot_look::state::RecordRead::Present(record) => Some(record.name),
            scoot_look::state::RecordRead::Missing | scoot_look::state::RecordRead::Corrupt(_) => {
                None
            }
        }
    }

    /// The picker marker: the configured glyph icon, else the default
    /// marker. A path or image icon marks nothing (the configured icon
    /// still draws in the bar text): popup rows are text the bar's fonts
    /// draw, as the power menu's rows are.
    #[cfg(feature = "popup")]
    fn marker(&self) -> char {
        match &self.icon {
            Some(Icon::Glyph(glyph)) => *glyph,
            _ => DEFAULT_MARKER,
        }
    }

    /// Carries out the picker's `N`th row: stages `scoot look apply NAME`
    /// for the loop to spawn, through the `scoot` resolved at start (an
    /// absolute path, so the spawn runs what the invoke checked). A row
    /// past what the picker showed is refused, never applied by position
    /// from a changed catalog; with no `scoot` on PATH the apply is
    /// refused too, so `invoke` answers an error instead of success for
    /// a spawn that cannot run.
    fn do_apply(&mut self, at: i32) -> Result<Update, InvokeError> {
        let index = usize::try_from(at).map_err(|_| InvokeError::Refused("no such look row"))?;
        let Some(name) = self.shown.get(index) else {
            return Err(InvokeError::Refused("no such look row"));
        };
        let Some(scoot) = &self.scoot else {
            return Err(InvokeError::Refused("no `scoot` on PATH to apply the look"));
        };
        self.staged = Some(Action::Exec(vec![
            scoot.to_string_lossy().into_owned(),
            "look".to_owned(),
            "apply".to_owned(),
            name.clone(),
        ]));
        Ok(Update::Unchanged)
    }

    /// Turns the picker to page `at`.
    fn do_page(&mut self, at: i32) -> Result<Update, InvokeError> {
        let page = usize::try_from(at).map_err(|_| InvokeError::Refused("no such page"))?;
        if page >= self.pages {
            return Err(InvokeError::Refused("no such page"));
        }
        if page == self.page {
            return Ok(Update::Unchanged);
        }
        self.page = page;
        Ok(Update::Changed)
    }

    /// Fills the picker from `names` (catalog order) with `active`
    /// selected: the rows of this page, with `‹ prev` and `next ›` rows
    /// where there is another page. The active row carries the marker
    /// before its name, as the power menu's row glyphs do; every other row
    /// keeps the marker's width in spaces, so the names align. The rows
    /// format into the popup's reused buffers (the scroll and
    /// draw paths allocate nothing); the `shown`
    /// snapshot keeps only this page, and the names list is rebuilt per
    /// fill — a popup fill is rare, never per frame.
    #[cfg(feature = "popup")]
    fn fill(
        &mut self,
        names: &[String],
        active: Option<&str>,
        content: &mut crate::popup::Content,
    ) -> bool {
        if names.is_empty() {
            self.shown.clear();
            self.pages = 0;
            self.page = 0;
            return false;
        }
        self.pages = names.len().div_ceil(PAGE_ROWS);
        if self.page >= self.pages {
            self.page = self.pages - 1;
        }
        let start = self.page * PAGE_ROWS;
        let end = (start + PAGE_ROWS).min(names.len());
        self.shown.clear();
        self.shown.extend(names[start..end].iter().cloned());
        if self.page > 0 {
            let pushed = content.button(
                format_args!("‹ prev"),
                "page",
                Some(self.page as i32 - 1),
                false,
                false,
            );
            if !pushed {
                return !content.is_empty();
            }
        }
        for (index, name) in self.shown.iter().enumerate() {
            let selected = Some(name.as_str()) == active;
            let marker = self.marker();
            let pushed = if selected {
                content.button(
                    format_args!("{marker} {name}"),
                    "apply",
                    Some(index as i32),
                    true,
                    true,
                )
            } else {
                content.button(
                    format_args!("{INACTIVE_PAD}{name}"),
                    "apply",
                    Some(index as i32),
                    false,
                    true,
                )
            };
            if !pushed {
                break;
            }
        }
        if self.page + 1 < self.pages {
            content.button(
                format_args!("next ›"),
                "page",
                Some(self.page as i32 + 1),
                false,
                false,
            );
        }
        !content.is_empty()
    }
}

impl Module for Looks {
    /// Nothing: no fd, no timer. The module wakes for nothing while it is
    /// only shown, so placing it costs no wakeups.
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        let _ = sources;
    }

    fn on_ready(&mut self, _source: usize, _events: PollFlags) -> Update {
        Update::Unchanged
    }

    /// The active look's name, read fresh from the state: always the last
    /// swap, whoever asked for it. `no look` with nothing recorded yet.
    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        if let Some(icon) = &self.icon {
            view.show_icon(icon);
        }
        match self.active() {
            Some(name) => {
                let _ = write!(view.text_mut(), "{name}");
                let _ = write!(view.tooltip_mut(), "Look: {name}");
            }
            None => {
                let _ = write!(view.text_mut(), "no look");
                let _ = write!(view.tooltip_mut(), "Look: none applied yet");
            }
        }
    }

    /// A click opens the picker; anything else means nothing. Without the
    /// `popup` feature a click means nothing either: there is no surface to
    /// open.
    fn on_input(&self, input: &Input<'_>) -> Option<Action> {
        match input.trigger {
            #[cfg(feature = "popup")]
            Trigger::Click => Some(Action::Module(ModuleAction::new(
                crate::action::POPUP,
                None,
            ))),
            #[cfg(not(feature = "popup"))]
            Trigger::Click => None,
            _ => None,
        }
    }

    /// Carries out the picker's rows and page turns: popup interactions and
    /// an agent's `invoke looks <action> N` land here alike.
    fn invoke(
        &mut self,
        _output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        let _ = steps;
        match &*action.name {
            "apply" => match action.arg {
                Some(at) => self.do_apply(at),
                None => Err(InvokeError::NeedsArg),
            },
            "page" => match action.arg {
                Some(at) => self.do_page(at),
                None => Err(InvokeError::NeedsArg),
            },
            _ => Err(InvokeError::Unknown),
        }
    }

    /// The staged `scoot look apply`, taken once: the loop carries it out
    /// like a binding.
    fn take_action(&mut self) -> Option<Action> {
        self.staged.take()
    }

    /// A click opens the picker with no binding at all (where there is a
    /// popup to open).
    fn handles_input(&self) -> bool {
        cfg!(feature = "popup")
    }

    /// The picker: the catalog in search order with the active look
    /// selected, paged past the popup's widget cap. Read fresh on every
    /// fill, so a swap from elsewhere moves the selection. Nothing with an
    /// empty catalog, which also closes an open picker.
    #[cfg(feature = "popup")]
    fn popup(&mut self, _output: &OutputView<'_>, content: &mut crate::popup::Content) -> bool {
        let catalog = scoot_look::catalog::discover();
        let active = self.active();
        let names: Vec<String> = catalog
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        self.fill(&names, active.as_deref(), content)
    }

    /// What `query` reports: the active look, the picker's page, and the
    /// names the picker last showed on it.
    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "active": self.active(),
            "page": self.page,
            "pages": self.pages,
            "looks": self.shown,
        }))
    }

    /// Its view carries a tooltip: the active look.
    #[cfg(feature = "popup")]
    fn tooltips(&self) -> bool {
        true
    }
}
