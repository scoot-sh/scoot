//! `outputs` and `[output."NAME"]`: what they take, and how they refuse.

use super::super::Error;
use super::super::tests::read;
use crate::bar::Edge;
use crate::policy::{MAX_OUTPUTS, Select};

fn refused(text: &str) -> String {
    match read(text) {
        Err(error) => error.to_string(),
        Ok(config) => panic!("accepted: {config:?}"),
    }
}

#[test]
fn outputs_defaults_to_all_and_takes_all_or_a_list() {
    assert_eq!(read("").unwrap().outputs.select, Select::All);
    assert_eq!(
        read("outputs = \"all\"").unwrap().outputs.select,
        Select::All
    );
    assert_eq!(
        read("outputs = [\"DP-1\", \"eDP-1\"]")
            .unwrap()
            .outputs
            .select,
        Select::Named(vec!["DP-1".into(), "eDP-1".into()])
    );
}

#[test]
fn a_bad_outputs_value_names_its_key() {
    for text in [
        "outputs = \"DP-1\"",
        "outputs = 3",
        "outputs = [1]",
        "outputs = [[\"DP-1\"]]",
        "outputs = []",
        "outputs = [\"\"]",
        "outputs = [\"DP-1\", \"DP-1\"]",
        "outputs = [\"a\\u001b[31m\"]",
    ] {
        let message = refused(text);
        assert!(message.contains("'outputs'"), "{text}: {message}");
    }
    // Never echoed raw: the escape is shown escaped or not at all.
    assert!(!refused("outputs = [\"a\\u001b[31m\"]").contains('\u{1b}'));
}

#[test]
fn too_many_outputs_are_refused_before_they_are_copied() {
    let list: Vec<String> = (0..=MAX_OUTPUTS).map(|n| format!("\"O-{n}\"")).collect();
    let message = refused(&format!("outputs = [{}]", list.join(",")));
    assert!(message.contains("at most"), "{message}");
    let tables: String = (0..=MAX_OUTPUTS)
        .map(|n| format!("[output.\"O-{n}\"]\nheight = 30\n"))
        .collect();
    assert!(refused(&tables).contains("'output'"));
}

#[test]
fn an_output_table_overrides_the_bar_keys_it_names() {
    let config = read(
        "[bar]\nheight = 28\n\
         [output.\"eDP-1\"]\nheight = 40\nedge = \"bottom\"\nlayer = \"overlay\"\n\
         exclusive = false\nmargin = \"2,4\"\n",
    )
    .unwrap();
    let [over] = &config.outputs.overrides[..] else {
        panic!("{:?}", config.outputs);
    };
    assert_eq!(over.name, "eDP-1");
    assert_eq!(over.bar.height, Some(40));
    assert_eq!(over.bar.edge, Some(Edge::Bottom));
    assert_eq!(over.bar.exclusive, Some(false));
    assert!(over.bar.margin.is_some());
    assert!(over.modules.is_none());
    // The shared bar is untouched.
    assert_eq!(config.bar.height, 28);
}

#[test]
fn a_bad_output_value_names_the_table_and_key() {
    for (text, key) in [
        ("height = 0", "height"),
        ("height = 100000", "height"),
        ("height = -1", "height"),
        ("edge = \"left\"", "edge"),
        ("layer = \"background\"", "layer"),
        ("margin = \"1,2,3,4,5\"", "margin"),
        ("left = [\"nope\"]", "left"),
    ] {
        let message = refused(&format!("[output.\"DP-1\"]\n{text}\n"));
        let want = format!("'output.\"DP-1\".{key}'");
        // A negative height is a TOML type refusal, which names the key
        // itself.
        assert!(
            message.contains(&want) || message.contains(key),
            "{text}: {message}"
        );
    }
}

#[test]
fn an_unknown_output_key_is_refused() {
    assert!(refused("[output.\"DP-1\"]\nheigth = 3\n").contains("heigth"));
    assert!(refused("outptus = \"all\"").contains("outptus"));
}

#[test]
fn an_output_table_for_an_unlisted_output_is_refused() {
    let message = refused("outputs = [\"DP-1\"]\n[output.\"DP-2\"]\nheight = 30\n");
    assert!(message.contains("DP-2") && message.contains("not in `outputs`"));
}

#[test]
fn an_output_table_with_a_hostile_name_is_refused() {
    let message = refused("[output.\"\"]\nheight = 30\n");
    assert!(message.contains("'output'"), "{message}");
    let message = refused("[output.\"a\\u001b[31m\"]\nheight = 30\n");
    assert!(!message.contains('\u{1b}'), "{message:?}");
}

#[cfg(any(feature = "clock", feature = "workspaces"))]
#[test]
fn module_lists_replace_the_whole_layout_per_output() {
    use super::super::tests::MODULE;
    let config = read(&format!(
        "[output.\"DP-1\"]\nright = [\"{MODULE}\"]\n\
         [output.\"DP-2\"]\nleft = []\n"
    ))
    .unwrap();
    let modules = |name: &str| {
        config
            .outputs
            .overrides
            .iter()
            .find(|o| o.name == name)
            .and_then(|o| o.modules.clone())
            .unwrap()
    };
    assert_eq!(modules("DP-1").right, [MODULE]);
    assert!(modules("DP-1").left.is_empty());
    // Giving any list sets all three: `left = []` is an empty bar.
    let empty = modules("DP-2");
    assert!(empty.left.is_empty() && empty.center.is_empty() && empty.right.is_empty());
    // The shared layout is untouched.
    assert_eq!(config.layout, crate::layout::Layout::default());
}

#[cfg(any(feature = "clock", feature = "workspaces"))]
#[test]
fn a_module_placed_twice_in_one_output_is_refused_but_not_across_outputs() {
    use super::super::tests::MODULE;
    let message = refused(&format!(
        "[output.\"DP-1\"]\nleft = [\"{MODULE}\"]\nright = [\"{MODULE}\"]\n"
    ));
    assert!(message.contains("output.\"DP-1\"") && message.contains("twice"));
    // The same module on two outputs is the point.
    assert!(
        read(&format!(
            "[output.\"DP-1\"]\nleft = [\"{MODULE}\"]\n[output.\"DP-2\"]\nleft = [\"{MODULE}\"]\n"
        ))
        .is_ok()
    );
}

#[test]
fn the_error_type_carries_the_table() {
    let Err(Error::Output { output, key, .. }) = read("[output.\"DP-1\"]\nheight = 0\n") else {
        panic!("not an output error");
    };
    assert_eq!((output.as_str(), key), ("DP-1", "height"));
}
