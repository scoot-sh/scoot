//! Key combinations written the way people and agents already write them:
//! `Return`, `ctrl+c`, `super+shift+Left`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Super,
}

impl Modifier {
    /// A modifier by name, case-insensitively, with the aliases a key combo
    /// accepts (`control`; `logo`, `meta`, `cmd` for Super). Public for a
    /// shell's config that names a modifier on its own (scoot's `[floating]
    /// modifier`), so both spell modifiers the same way.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Self::Ctrl),
            "shift" => Some(Self::Shift),
            "alt" => Some(Self::Alt),
            "super" | "logo" | "meta" | "cmd" => Some(Self::Super),
            _ => None,
        }
    }

    /// The spelling `parse` accepts and [`KeyCombo`]'s `Display` writes --
    /// also what a compositor should name in an error about this modifier,
    /// so the message quotes back what the caller wrote.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ctrl => "ctrl",
            Self::Shift => "shift",
            Self::Alt => "alt",
            Self::Super => "super",
        }
    }
}

/// A key plus the modifiers held while pressing it. `key` is an xkb keysym name
/// (`Return`, `a`, `F5`); each shell resolves it against its own keymap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct KeyCombo {
    pub modifiers: Vec<Modifier>,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseKeyComboError {
    MissingKey,
    UnknownModifier(String),
}

impl fmt::Display for ParseKeyComboError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingKey => f.write_str("key combination has no key"),
            Self::UnknownModifier(name) => write!(f, "unknown modifier `{name}`"),
        }
    }
}

impl std::error::Error for ParseKeyComboError {}

impl FromStr for KeyCombo {
    type Err = ParseKeyComboError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let key = parts
            .pop()
            .filter(|key| !key.is_empty())
            .ok_or(ParseKeyComboError::MissingKey)?;
        let modifiers = parts
            .into_iter()
            .map(|name| {
                Modifier::parse(name)
                    .ok_or_else(|| ParseKeyComboError::UnknownModifier(name.to_owned()))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            modifiers,
            key: key.to_owned(),
        })
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for modifier in &self.modifiers {
            write!(f, "{}+", modifier.name())?;
        }
        f.write_str(&self.key)
    }
}

impl TryFrom<String> for KeyCombo {
    type Error = ParseKeyComboError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<KeyCombo> for String {
    fn from(combo: KeyCombo) -> Self {
        combo.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_and_key() {
        let combo: KeyCombo = "Ctrl+shift+T".parse().unwrap();
        assert_eq!(combo.modifiers, vec![Modifier::Ctrl, Modifier::Shift]);
        assert_eq!(combo.key, "T");
    }

    #[test]
    fn a_bare_key_has_no_modifiers() {
        let combo: KeyCombo = "Return".parse().unwrap();
        assert!(combo.modifiers.is_empty());
        assert_eq!(combo.to_string(), "Return");
    }

    #[test]
    fn display_round_trips_in_canonical_form() {
        let combo: KeyCombo = "control + cmd + Left".parse().unwrap();
        assert_eq!(combo.to_string(), "ctrl+super+Left");
        assert_eq!(combo.to_string().parse::<KeyCombo>().unwrap(), combo);
    }

    #[test]
    fn rejects_missing_keys_and_unknown_modifiers() {
        assert_eq!("".parse::<KeyCombo>(), Err(ParseKeyComboError::MissingKey));
        assert_eq!(
            "ctrl+".parse::<KeyCombo>(),
            Err(ParseKeyComboError::MissingKey)
        );
        assert_eq!(
            "hyper+a".parse::<KeyCombo>(),
            Err(ParseKeyComboError::UnknownModifier("hyper".into()))
        );
    }
}
