//! Each kind of table on its own: what a build with only that feature reads,
//! and that the other two kinds' tables are refused there as unknown.

#[cfg(any(feature = "button", feature = "push", feature = "exec"))]
use super::tests::read;

#[cfg(feature = "button")]
#[test]
fn a_button_table_is_read() {
    let config = read("center = [\"b\"]\n[button.b]\ntext = \"x\"\n").unwrap();
    assert_eq!(config.layout.center, ["b"]);
    assert_eq!(config.modules.custom.len(), 1);
}

#[cfg(feature = "push")]
#[test]
fn a_push_table_is_read() {
    let config = read("center = [\"p\"]\n[push.p]\nplaceholder = \"x\"\n").unwrap();
    assert_eq!(config.layout.center, ["p"]);
    assert_eq!(config.modules.custom.len(), 1);
}

#[cfg(feature = "exec")]
#[test]
fn an_exec_table_is_read() {
    let config = read("center = [\"e\"]\n[exec.e]\ncommand = [\"true\"]\n").unwrap();
    assert_eq!(config.layout.center, ["e"]);
    assert_eq!(config.modules.custom.len(), 1);
}

#[cfg(not(feature = "button"))]
#[test]
fn without_the_button_feature_its_table_is_unknown() {
    let error = super::tests::read("[button.b]\n").unwrap_err().to_string();
    assert!(error.contains("button"), "{error}");
}

#[cfg(not(feature = "push"))]
#[test]
fn without_the_push_feature_its_table_is_unknown() {
    let error = super::tests::read("[push.p]\n").unwrap_err().to_string();
    assert!(error.contains("push"), "{error}");
}

#[cfg(not(feature = "exec"))]
#[test]
fn without_the_exec_feature_its_table_is_unknown() {
    let error = super::tests::read("[exec.e]\n").unwrap_err().to_string();
    assert!(error.contains("exec"), "{error}");
}
