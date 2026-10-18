//! Desktop actions: Desktop Entry §11.
//!
//! `Actions` names the action ids; `Desktop Action <id>` groups not
//! named in `Actions` are ignored, as are actions missing `Name` or
//! `Exec`. Names are localized in the spec's order.

#[cfg(test)]
mod tests;

/// One action's raw material; the caller ([`super::index`](crate::index))
/// expands the `Exec` with the same rules as the main one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionDef {
    pub id: String,
    pub name: String,
    pub exec: String,
}

/// Collects an entry's actions, in `Actions` order. At most
/// [`MAX_ACTIONS`] per file; past that the first win, with a note.
pub const MAX_ACTIONS: usize = 16;

pub fn collect(
    file: &super::parse::ParsedFile,
    locale: Option<&super::locale::Locale>,
    notes: &mut Vec<String>,
    id: &str,
) -> Vec<ActionDef> {
    let Some(main) = file.groups.first() else {
        return Vec::new();
    };
    let Some(raw) = main.get("Actions") else {
        return Vec::new();
    };
    let ids = match super::parse::split_list(raw) {
        Ok(ids) => ids,
        Err(_) => {
            // A parse error for that key: the key is ignored, the
            // entry kept — with no actions.
            return Vec::new();
        }
    };
    let mut actions = Vec::new();
    for action_id in ids {
        if actions.len() >= MAX_ACTIONS {
            notes.push(format!(
                "{id}: more than {MAX_ACTIONS} actions, the rest ignored"
            ));
            break;
        }
        let group_name = format!("Desktop Action {action_id}");
        let Some(group) = file.groups.iter().find(|group| group.name == group_name) else {
            notes.push(format!(
                "{id}: action `{action_id}` has no [{group_name}] group, ignored"
            ));
            continue;
        };
        let lookup = |key: &str| group.get(key);
        let Some(name) = super::locale::get_localized(lookup, "Name", locale) else {
            notes.push(format!("{id}: action `{action_id}` has no Name, ignored"));
            continue;
        };
        let name = super::parse::clean_display(&name);
        if name.is_empty() {
            notes.push(format!(
                "{id}: action `{action_id}` has an empty Name, ignored"
            ));
            continue;
        }
        let Some(exec) = group.get("Exec") else {
            notes.push(format!("{id}: action `{action_id}` has no Exec, ignored"));
            continue;
        };
        actions.push(ActionDef {
            id: action_id,
            name,
            exec: exec.to_string(),
        });
    }
    actions
}
