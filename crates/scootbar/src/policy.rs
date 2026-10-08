//! Which outputs get a bar, and how each one's differs from the rest: the
//! `outputs` list and the `[output."NAME"]` overrides. Pure, so every
//! choice is a unit test.
//!
//! - **Selection.** [`Select::All`] (the default) gives every output a bar;
//!   [`Select::Named`] only the outputs whose `wl_output.name` (the
//!   connector, `DP-1`) is listed. An output the compositor never named (a
//!   `wl_output` older than v4) matches only `all`. The decision is made at
//!   the output's settle, when its name is known, and again on a reload, so
//!   a monitor plugged in later is judged the same way as one there at
//!   start-up.
//! - **Overrides.** An output named in `[output."NAME"]` may change its bar's
//!   `edge`, `layer`, `exclusive`, `height` and `margin`, and set its own
//!   module lists. An override is more specific than any flag: `--height`
//!   sets the height of the outputs that override none. Style (colors, font,
//!   padding, spacing) and each module's own options stay shared by every
//!   bar, as does the one set of started modules: an override chooses which
//!   of them an output shows and where, never starts a second one.
//! - **What a name is.** Anything the compositor calls an output, as text:
//!   compared byte for byte, never interpreted.

use std::fmt;

use crate::bar::{Bar, Edge, Layer, Margin};
use crate::layout::{Layout, Section, is_separator};

#[cfg(test)]
mod tests;

/// The most outputs an `outputs` list names, and the most `[output]`
/// tables: more is not a setup, and it bounds what a hostile file costs.
pub const MAX_OUTPUTS: usize = 32;
/// The longest output name taken, in bytes (connector names are a dozen).
pub const MAX_NAME: usize = 128;

/// Which outputs have a bar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Select {
    #[default]
    All,
    /// Exactly these connector names.
    Named(Vec<String>),
}

/// Why an output name is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    /// A control character: never a connector name, and not to be echoed
    /// to a terminal.
    Control,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "an output name is not empty"),
            Self::TooLong => write!(f, "an output name is at most {MAX_NAME} bytes"),
            Self::Control => write!(f, "an output name has no control characters"),
        }
    }
}

/// `Ok` for a name that can be an output's.
pub fn check_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        Err(NameError::Empty)
    } else if name.len() > MAX_NAME {
        Err(NameError::TooLong)
    } else if name.chars().any(char::is_control) {
        Err(NameError::Control)
    } else {
        Ok(())
    }
}

/// The bar geometry an output changes: each `None` keeps the shared value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BarOverride {
    pub edge: Option<Edge>,
    pub layer: Option<Layer>,
    pub exclusive: Option<bool>,
    pub height: Option<u32>,
    pub margin: Option<Margin>,
}

impl BarOverride {
    fn apply(&self, bar: &Bar) -> Bar {
        Bar {
            edge: self.edge.unwrap_or(bar.edge),
            layer: self.layer.unwrap_or(bar.layer),
            exclusive: self.exclusive.unwrap_or(bar.exclusive),
            height: self.height.unwrap_or(bar.height),
            margin: self.margin.unwrap_or(bar.margin),
        }
    }
}

/// An output's own module lists: all three sections together, as the
/// shared ones are (a section not given is empty).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sections {
    pub left: Vec<&'static str>,
    pub center: Vec<&'static str>,
    pub right: Vec<&'static str>,
}

/// One `[output."NAME"]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Override {
    pub name: String,
    pub bar: BarOverride,
    pub modules: Option<Sections>,
}

/// The `outputs` list and the overrides.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Policy {
    pub select: Select,
    pub overrides: Vec<Override>,
}

/// Why a policy is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    /// `outputs = []`: no bar anywhere is `scootbar msg hide`, not a list.
    NoOutputs,
    TooManyOutputs,
    Name(NameError),
    /// Listed twice.
    Twice(String),
    /// An override for an output the `outputs` list leaves out: it could
    /// never apply.
    Unselected(String),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOutputs => write!(
                f,
                "lists no output: name at least one, or \"all\" (to show no bar at all, \
                 use `scootbar msg hide`)"
            ),
            Self::TooManyOutputs => write!(f, "at most {MAX_OUTPUTS} outputs"),
            Self::Name(error) => write!(f, "{error}"),
            Self::Twice(name) => write!(f, "`{}` is listed twice", name.escape_debug()),
            Self::Unselected(name) => write!(
                f,
                "`{}` has an [output] table but is not in `outputs`, so it would never get a bar",
                name.escape_debug()
            ),
        }
    }
}

/// What one output gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub selected: bool,
    pub bar: Bar,
    /// The modules it shows and where (its own lists, or the shared ones);
    /// the gaps are the shared ones.
    pub layout: Layout,
}

/// The shared bar and layout, and the policy over them: everything that
/// decides what one output gets, as the daemon keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub bar: Bar,
    pub layout: Layout,
    pub policy: Policy,
}

impl Placement {
    /// What the output called `name` (if the compositor named it) gets.
    /// Cold path: a settle, a reload.
    pub fn resolve(&self, name: Option<&str>) -> Resolved {
        self.policy.resolve(name, &self.bar, &self.layout)
    }
}

impl Policy {
    /// Whether the output called `name` (if the compositor named it) has a
    /// bar.
    pub fn selects(&self, name: Option<&str>) -> bool {
        match (&self.select, name) {
            (Select::All, _) => true,
            (Select::Named(list), Some(name)) => list.iter().any(|listed| listed == name),
            (Select::Named(_), None) => false,
        }
    }

    /// The list is a usable one: not empty, no duplicates, sane names, and
    /// every override on an output that is selected. Run after the flags
    /// are overlaid too, since `--outputs` can clash with the file's
    /// tables.
    pub fn check(&self) -> Result<(), PolicyError> {
        if let Select::Named(list) = &self.select {
            if list.is_empty() {
                return Err(PolicyError::NoOutputs);
            }
            if list.len() > MAX_OUTPUTS {
                return Err(PolicyError::TooManyOutputs);
            }
            for (index, name) in list.iter().enumerate() {
                check_name(name).map_err(PolicyError::Name)?;
                if list.iter().take(index).any(|earlier| earlier == name) {
                    return Err(PolicyError::Twice(name.clone()));
                }
            }
        }
        if self.overrides.len() > MAX_OUTPUTS {
            return Err(PolicyError::TooManyOutputs);
        }
        for over in &self.overrides {
            check_name(&over.name).map_err(PolicyError::Name)?;
            if !self.selects(Some(&over.name)) {
                return Err(PolicyError::Unselected(over.name.clone()));
            }
        }
        Ok(())
    }

    fn override_for(&self, name: Option<&str>) -> Option<&Override> {
        let name = name?;
        self.overrides.iter().find(|over| over.name == name)
    }

    /// What the output called `name` gets, given the shared `bar` and
    /// `layout`. Cold path: a settle, a reload.
    pub fn resolve(&self, name: Option<&str>, bar: &Bar, layout: &Layout) -> Resolved {
        let over = self.override_for(name);
        Resolved {
            selected: self.selects(name),
            bar: over.map_or(*bar, |over| over.bar.apply(bar)),
            layout: match over.and_then(|over| over.modules.as_ref()) {
                Some(own) => Layout {
                    left: own.left.clone(),
                    center: own.center.clone(),
                    right: own.right.clone(),
                    padding: layout.padding,
                    spacing: layout.spacing,
                    separator: layout.separator,
                    margins: layout.margins.clone(),
                },
                None => layout.clone(),
            },
        }
    }

    /// Every module any output may show, each once, for the one start-up:
    /// the shared layout's, then the overrides' that it lacks, in file
    /// order. The sections are where each was first placed; each output's
    /// own placement is [`Policy::resolve`]'s. A `"|"` mark is not a
    /// module: it is never started, so it is left out.
    pub fn to_start(&self, layout: &Layout) -> Layout {
        let mut all = layout.clone();
        for over in &self.overrides {
            let Some(own) = &over.modules else { continue };
            for (section, ids) in [
                (Section::Left, &own.left),
                (Section::Center, &own.center),
                (Section::Right, &own.right),
            ] {
                for &id in ids {
                    if is_separator(id) {
                        continue;
                    }
                    if all.placed().any(|(_, placed)| placed == id) {
                        continue;
                    }
                    match section {
                        Section::Left => all.left.push(id),
                        Section::Center => all.center.push(id),
                        Section::Right => all.right.push(id),
                    }
                }
            }
        }
        all
    }
}
