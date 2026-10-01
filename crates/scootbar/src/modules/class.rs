//! A state class and the text bound: the two things a module's payload
//! shares with the view. Pure and dependency-free (`std` only), so the
//! fuzz crate compiles this file and `payload.rs` unchanged
//! (`crates/scootbar/fuzz`).

/// A state class: how the bar colors a module's view (through the theme's
/// tokens, `crate::theme`). A module never picks colors itself. The clock
/// is always `Normal`; the others are the contract's, for the modules that
/// follow it (battery, volume), and tested through the render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum Class {
    #[default]
    Normal,
    Warn,
    Urgent,
    Muted,
}

impl Class {
    #[allow(dead_code)]
    /// The class named `name` (`normal`, `warn`, `urgent` or `muted`), as
    /// `name` prints it.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Normal, Self::Warn, Self::Urgent, Self::Muted]
            .into_iter()
            .find(|class| class.name() == name)
    }

    /// The class's name in a `query` reply: `normal`, `warn`, `urgent` or
    /// `muted`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Warn => "warn",
            Self::Urgent => "urgent",
            Self::Muted => "muted",
        }
    }
}

/// The most text a view holds, in bytes; more is cut at a character
/// boundary. Bounds what an untrusted string (a window title) can cost.
pub const MAX_TEXT: usize = 256;
