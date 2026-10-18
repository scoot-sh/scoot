//! The `Notify` body to a bounded [`Incoming`]: the spec's §9.1.2 eight
//! arguments, every one capped.
//!
//! Field caps (cut at a char boundary, never refused): `app_name` 128 B,
//! `summary` 1 KiB, `body` 8 KiB, an action key 64 B, an action label
//! 128 B, at most 16 action pairs read, at most 64 hints read (the rest
//! skipped unread). Control characters other than `\n` and `\t` are
//! dropped at parse. A body whose signature is not the spec's
//! `susssasa{sv}i` is `InvalidArgs`, never a guess.

use scoot_dbus::proto::Reader;

use super::hints::{self, Hints, cut};

/// `app_name` past this is cut, never refused.
pub const MAX_APP_NAME: usize = 128;
/// `summary` past this is cut: summaries wrap to two lines, not novels.
pub const MAX_SUMMARY: usize = 1024;
/// `body` past this is cut: five lines draw from far less.
pub const MAX_BODY: usize = 8 * 1024;
/// An action key past this is cut.
pub const MAX_ACTION_KEY: usize = 64;
/// An action label past this is cut.
pub const MAX_ACTION_LABEL: usize = 128;
/// Action pairs past 16 are not read (4 draw as buttons).
pub const MAX_ACTION_PAIRS: usize = 16;
/// `app_icon` past this is cut: a reference, not prose.
pub const MAX_APP_ICON: usize = 1024;

/// The spec's `Notify` body signature: all eight arguments, in order.
pub const NOTIFY_SIGNATURE: &str = "susssasa{sv}i";

/// One action pair: `default` is never a button, but the card body
/// invokes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub key: String,
    pub label: String,
}

/// A bounded `Notify`: everything the store keeps, nothing it must not.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub app_name: String,
    /// 0, or the id to atomically replace.
    pub replaces_id: u32,
    /// `app_icon`: a `file://` URI, an absolute path, or a theme name.
    pub app_icon: String,
    pub summary: String,
    /// The raw body: markup parses at draw time, plain text in JSON.
    pub body: String,
    /// At most [`MAX_ACTION_PAIRS`] pairs, in order.
    pub actions: Vec<Action>,
    pub hints: Hints,
    pub expire_timeout: i32,
    /// The caller's unique name: the replaces policy and the rate
    /// limiter key on it.
    pub sender: String,
}

/// Parses the `Notify` body at `reader` (whose message signature the
/// caller already matched): `Err` is `InvalidArgs` — a misshapen body, a
/// short read, or trailing bytes past the eight arguments. Never panics.
pub fn parse(reader: &mut Reader<'_>, sender: &str) -> Result<Incoming, ()> {
    let app_name = capped_str(reader, MAX_APP_NAME)?;
    let replaces_id = reader.u32()?;
    let app_icon = capped_str(reader, MAX_APP_ICON)?;
    let summary = capped_str(reader, MAX_SUMMARY)?;
    let body = capped_str(reader, MAX_BODY)?;
    let actions = read_actions(reader)?;
    let hints = hints::read(reader)?;
    let expire_timeout = reader.i32()?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok(Incoming {
        app_name,
        replaces_id,
        app_icon,
        summary,
        body,
        actions,
        hints,
        expire_timeout,
        sender: sender.to_owned(),
    })
}

/// One capped string: cut at a char boundary, controls dropped.
fn capped_str(reader: &mut Reader<'_>, max: usize) -> Result<String, ()> {
    let text = reader.str()?;
    Ok(drop_controls(&cut(text, max)))
}

/// The action array to at most [`MAX_ACTION_PAIRS`] pairs: a trailing odd
/// string is ignored (the spec pairs keys with labels).
fn read_actions(reader: &mut Reader<'_>) -> Result<Vec<Action>, ()> {
    let mut elements = reader.elements(4)?;
    let mut strings = Vec::new();
    while !elements.exhausted() {
        strings.push(elements.str()?.to_owned());
    }
    let mut actions = Vec::new();
    let mut at = 0;
    while at + 1 < strings.len() && actions.len() < MAX_ACTION_PAIRS {
        actions.push(Action {
            key: cut(&drop_controls(&strings[at]), MAX_ACTION_KEY),
            label: cut(&drop_controls(&strings[at + 1]), MAX_ACTION_LABEL),
        });
        at += 2;
    }
    Ok(actions)
}

/// Drops control characters other than `\n` and `\t`: they never reach
/// the screen, and a bell in a summary is a prank, not content.
pub fn drop_controls(text: &str) -> String {
    if !text
        .bytes()
        .any(|byte| (byte < 0x20 && !matches!(byte, b'\n' | b'\t')) || byte == 0x7f)
    {
        return text.to_owned();
    }
    text.chars()
        .filter(|char| !matches!(*char, '\0'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}'))
        .collect()
}

/// The fuzz target's check over arbitrary bytes: a `Notify` body, refused
/// or bounded without a panic. Whatever parses keeps every cap the
/// module documents.
pub fn fuzz_check(bytes: &[u8]) -> bool {
    let mut reader = Reader::le(bytes);
    let Ok(incoming) = parse(&mut reader, ":1.1") else {
        return false;
    };
    assert!(incoming.app_name.len() <= MAX_APP_NAME, "app_name");
    assert!(incoming.app_icon.len() <= MAX_APP_ICON, "app_icon");
    assert!(incoming.summary.len() <= MAX_SUMMARY, "summary");
    assert!(incoming.body.len() <= MAX_BODY, "body");
    assert!(incoming.actions.len() <= MAX_ACTION_PAIRS, "actions");
    for action in &incoming.actions {
        assert!(action.key.len() <= MAX_ACTION_KEY, "key");
        assert!(action.label.len() <= MAX_ACTION_LABEL, "label");
    }
    assert!(incoming.hints.category.len() <= super::hints::MAX_SHORT_HINT, "category");
    assert!(
        incoming.hints.desktop_entry.len() <= super::hints::MAX_SHORT_HINT,
        "desktop-entry"
    );
    true
}

#[cfg(test)]
mod tests;
