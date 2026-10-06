//! `Action::ShowKeymap` expands to the shell directive, naming nothing:
//! no terminal, no shell, no pager -- those are the shell's to choose (see
//! the `scoot` crate's `show_keymap` module), keeping this crate
//! platform-independent.

use crate::{Action, Config, Effect, World};

#[test]
fn show_keymap_expands_to_the_shell_directive() {
    let mut world = World::new(Config::default());
    assert_eq!(
        world.handle_action(Action::ShowKeymap),
        vec![Effect::ShowKeymap]
    );
}
