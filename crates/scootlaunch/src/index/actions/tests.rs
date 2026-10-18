//! Tests for actions: §11's ignore rules and localization.

use super::*;

fn file(text: &str) -> super::super::parse::ParsedFile {
    super::super::parse::parse(text.as_bytes()).unwrap()
}

fn collect_names(text: &str) -> Vec<(String, String)> {
    let file = file(text);
    let mut notes = Vec::new();
    collect(&file, None, &mut notes, "test.desktop")
        .into_iter()
        .map(|action| (action.id, action.name))
        .collect()
}

#[test]
fn named_actions_with_name_and_exec_collect_in_order() {
    let names = collect_names(
        "[Desktop Entry]\nName=Foo\nExec=foo\nActions=second;first;\n\
         [Desktop Action first]\nName=First\nExec=foo --first\n\
         [Desktop Action second]\nName=Second\nExec=foo --second\n",
    );
    assert_eq!(
        names,
        vec![
            ("second".to_string(), "Second".to_string()),
            ("first".to_string(), "First".to_string()),
        ]
    );
}

#[test]
fn unnamed_groups_are_ignored() {
    let names = collect_names(
        "[Desktop Entry]\nName=Foo\nExec=foo\nActions=real;\n\
         [Desktop Action real]\nName=Real\nExec=foo\n\
         [Desktop Action stray]\nName=Stray\nExec=stray\n",
    );
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].0, "real");
}

#[test]
fn actions_missing_name_or_exec_are_ignored() {
    let file = file(
        "[Desktop Entry]\nName=Foo\nExec=foo\nActions=noexec;noname;ok;\n\
         [Desktop Action noexec]\nName=Noexec\n\
         [Desktop Action noname]\nExec=foo\n\
         [Desktop Action ok]\nName=Ok\nExec=foo\n",
    );
    let mut notes = Vec::new();
    let actions = collect(&file, None, &mut notes, "test.desktop");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "ok");
    assert_eq!(notes.len(), 2);
}

#[test]
fn no_actions_key_means_no_actions() {
    let file =
        file("[Desktop Entry]\nName=Foo\nExec=foo\n[Desktop Action stray]\nName=S\nExec=s\n");
    let mut notes = Vec::new();
    assert!(collect(&file, None, &mut notes, "test.desktop").is_empty());
    assert!(notes.is_empty());
}

#[test]
fn action_names_are_localized() {
    let file = file(
        "[Desktop Entry]\nName=Foo\nExec=foo\nActions=open;\n\
         [Desktop Action open]\nName=Open\nName[de]=Öffnen\nExec=foo\n",
    );
    let mut notes = Vec::new();
    let locale = super::super::locale::parse_locale("de").unwrap();
    let actions = collect(&file, Some(&locale), &mut notes, "test.desktop");
    assert_eq!(actions[0].name, "Öffnen");
}

#[test]
fn actions_are_capped_with_a_note() {
    let mut text = String::from("[Desktop Entry]\nName=Foo\nExec=foo\nActions=");
    for index in 0..20 {
        text.push_str(&format!("a{index};"));
    }
    text.push('\n');
    for index in 0..20 {
        text.push_str(&format!(
            "[Desktop Action a{index}]\nName=A{index}\nExec=foo\n"
        ));
    }
    let file = file(&text);
    let mut notes = Vec::new();
    let actions = collect(&file, None, &mut notes, "test.desktop");
    assert_eq!(actions.len(), MAX_ACTIONS);
    assert!(notes.iter().any(|note| note.contains("more than")));
}
