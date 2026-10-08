//! The config file's multi-output keys: the top-level `outputs` and the
//! `[output."NAME"]` tables, validated into a [`Policy`]. Each value is
//! checked the way its `[bar]` twin is, and an error names the table.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::Deserialize;

#[cfg(test)]
mod tests;

use super::{Error, MAX_FONT_SIZE, ids, parse_margin};
use crate::bar::{Edge, Layer, MAX_HEIGHT};
use crate::layout::{Section, check_placement};
use crate::policy::{
    BarOverride, MAX_OUTPUTS, Override, Policy, PolicyError, Sections, Select, check_name,
};

/// One `[output."NAME"]` table: the bar keys that may differ per output,
/// and the module lists.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct OutputFile {
    edge: Option<String>,
    layer: Option<String>,
    exclusive: Option<bool>,
    height: Option<u32>,
    margin: Option<toml::Value>,
    #[serde(rename = "font-size")]
    font_size: Option<u32>,
    left: Option<Vec<String>>,
    center: Option<Vec<String>>,
    right: Option<Vec<String>>,
}

/// A value refused in `[output."NAME"]`.
pub fn refusal(path: &Path, output: &str, key: &'static str, message: impl fmt::Display) -> Error {
    Error::Output {
        path: path.to_owned(),
        output: output.to_owned(),
        key,
        message: message.to_string(),
    }
}

/// Validates the keys into a [`Policy`], or names what is wrong.
pub(super) fn policy(
    path: &Path,
    outputs: Option<&toml::Value>,
    tables: &BTreeMap<String, OutputFile>,
    known: &[&'static str],
) -> Result<Policy, Error> {
    let value = |message: PolicyError| Error::Value {
        path: path.to_owned(),
        key: "outputs",
        message: message.to_string(),
    };
    let select = match outputs {
        None => Select::All,
        Some(toml::Value::String(text)) if text == "all" => Select::All,
        Some(toml::Value::Array(items)) => {
            // Bounded before it is copied: the file is capped, but a
            // list of thousands of empty strings is still cheap to refuse.
            if items.len() > MAX_OUTPUTS {
                return Err(value(PolicyError::TooManyOutputs));
            }
            let mut names = Vec::with_capacity(items.len());
            for item in items {
                let toml::Value::String(name) = item else {
                    return Err(Error::Value {
                        path: path.to_owned(),
                        key: "outputs",
                        message: "takes \"all\" or a list of connector names (strings)".to_owned(),
                    });
                };
                names.push(name.clone());
            }
            Select::Named(names)
        }
        Some(_) => {
            return Err(Error::Value {
                path: path.to_owned(),
                key: "outputs",
                message: "takes \"all\" or a list of connector names, like [\"DP-1\"]".to_owned(),
            });
        }
    };
    if tables.len() > MAX_OUTPUTS {
        return Err(Error::Value {
            path: path.to_owned(),
            key: "output",
            message: PolicyError::TooManyOutputs.to_string(),
        });
    }
    let mut overrides = Vec::with_capacity(tables.len());
    for (name, table) in tables {
        check_name(name).map_err(|error| Error::Value {
            path: path.to_owned(),
            key: "output",
            message: format!("`{}`: {error}", name.escape_debug()),
        })?;
        overrides.push(over(path, name, table, known)?);
    }
    let policy = Policy { select, overrides };
    policy.check().map_err(|error| Error::Value {
        path: path.to_owned(),
        key: match error {
            PolicyError::Unselected(_) => "output",
            _ => "outputs",
        },
        message: error.to_string(),
    })?;
    Ok(policy)
}

fn over(
    path: &Path,
    name: &str,
    table: &OutputFile,
    known: &[&'static str],
) -> Result<Override, Error> {
    let edge = match &table.edge {
        None => None,
        Some(text) => Some(Edge::parse(text).ok_or_else(|| {
            refusal(
                path,
                name,
                "edge",
                format_args!("takes top or bottom, not `{}`", text.escape_debug()),
            )
        })?),
    };
    let layer = match &table.layer {
        None => None,
        Some(text) => Some(Layer::parse(text).ok_or_else(|| {
            refusal(
                path,
                name,
                "layer",
                format_args!(
                    "takes bottom, top or overlay, not `{}`",
                    text.escape_debug()
                ),
            )
        })?),
    };
    let height = match table.height {
        None => None,
        Some(height) if (1..=MAX_HEIGHT).contains(&height) => Some(height),
        Some(height) => {
            return Err(refusal(
                path,
                name,
                "height",
                format_args!(
                    "takes a whole number of logical pixels from 1 to {MAX_HEIGHT}, \
                     not `{height}`"
                ),
            ));
        }
    };
    let margin = match &table.margin {
        None => None,
        Some(margin) => Some(
            parse_margin(margin)
                .map_err(|message| refusal(path, name, "margin", format_args!("{message}")))?,
        ),
    };
    let font_size = match table.font_size {
        None => None,
        Some(size) if (1..=MAX_FONT_SIZE).contains(&size) => Some(size),
        Some(size) => {
            return Err(refusal(
                path,
                name,
                "font-size",
                format_args!(
                    "takes a whole number of logical pixels from 1 to {MAX_FONT_SIZE}, \
                     not `{size}`"
                ),
            ));
        }
    };
    let modules = if table.left.is_none() && table.center.is_none() && table.right.is_none() {
        None
    } else {
        let list = |section: Section, given: &Option<Vec<String>>| {
            ids(path, section, given, known).map_err(|error| match error {
                // The shared list's error names `left`, not the table.
                Error::Value { key, message, .. } => refusal(path, name, key, message),
                other => other,
            })
        };
        let sections = Sections {
            left: list(Section::Left, &table.left)?,
            center: list(Section::Center, &table.center)?,
            right: list(Section::Right, &table.right)?,
        };
        let layout = crate::layout::Layout {
            left: sections.left.clone(),
            center: sections.center.clone(),
            right: sections.right.clone(),
            ..Default::default()
        };
        check_placement(&layout)
            .map_err(|(section, error)| refusal(path, name, section.name(), error.to_string()))?;
        Some(sections)
    };
    Ok(Override {
        name: name.to_owned(),
        bar: BarOverride {
            edge,
            layer,
            exclusive: table.exclusive,
            height,
            margin,
        },
        font_size,
        modules,
    })
}
