//! `Action::ShowKeymap` expands to the spawn carrying the pager pipeline:
//! the default terminal running the live keymap, not arrangement.

use crate::{Action, Config, Effect, World};

#[test]
fn show_keymap_expands_to_the_terminal_pager_spawn() {
    let mut world = World::new(Config::default());
    assert_eq!(
        world.handle_action(Action::ShowKeymap),
        vec![Effect::Spawn(vec![
            "foot".to_owned(),
            "sh".to_owned(),
            "-c".to_owned(),
            "scoot msg binds | less".to_owned(),
        ])]
    );
}
