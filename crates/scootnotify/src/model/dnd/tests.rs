//! The hold test: DND starts off, toggles, and names itself.

use super::Dnd;

#[test]
fn dnd_starts_off_and_toggles() {
    assert_eq!(Dnd::default(), Dnd::Off);
    assert_eq!(Dnd::Off.name(), "off");
    assert_eq!(Dnd::On.name(), "on");
    assert!(!Dnd::Off.holds());
    assert!(Dnd::On.holds());
}
