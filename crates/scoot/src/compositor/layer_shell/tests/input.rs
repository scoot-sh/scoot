//! Where input goes when a layer surface is on screen.
//!
//! The pointer half -- a bar is clickable, and clicking one does not refocus
//! the window behind it -- and the keyboard half, which is most of this file:
//! who holds the keyboard, when it changes hands, and what the client is told
//! about it.

use super::*;

// -------------------------------------------------------------------------
// Input
// -------------------------------------------------------------------------

/// The pointer reaches a bar rather than the window behind it, and still
/// reaches the window everywhere else. Without this a bar draws but cannot
/// be clicked, which is most of what a bar is for.
#[test]
fn the_pointer_finds_a_bar_in_front_of_a_window() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec {
        exclusive_zone: 0,
        ..LayerSpec::bar(30)
    }));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let rect = fixture.window_rect();
    let window_surface = fixture
        .state
        .windows
        .values()
        .next()
        .and_then(|window| window.toplevel().map(|t| t.wl_surface().clone()))
        .expect("a mapped window");

    // Over the bar, in a column the window also occupies.
    let over_bar = ((rect.x + 5) as f64, 15.0).into();
    let (found, _) = fixture
        .state
        .surface_under(over_bar)
        .expect("something under the pointer at the bar");
    assert_ne!(found, window_surface, "the window swallowed a bar click");
    assert!(
        fixture
            .state
            .layer_surface_under(&ABOVE_WINDOWS, over_bar)
            .is_some(),
        "the bar should be found on the layers above windows"
    );
    assert!(
        fixture
            .state
            .layer_surface_under(&BELOW_WINDOWS, over_bar)
            .is_none(),
        "a top-layer bar must not answer for the layers below windows"
    );

    // Below the bar, the window itself still answers.
    let over_window = ((rect.x + 5) as f64, (rect.y + 25) as f64).into();
    let (found, _) = fixture
        .state
        .surface_under(over_window)
        .expect("something under the pointer at the window");
    assert_eq!(found, window_surface);
}

/// Clicking a bar must not activate the window behind it. The control half
/// -- clicking the window itself -- is what keeps this from passing
/// vacuously on a compositor that never focuses anything.
#[test]
fn clicking_a_bar_does_not_refocus_the_window_behind_it() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec {
        exclusive_zone: 0,
        ..LayerSpec::bar(30)
    }));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let placements = fixture.state.world.arrange().placements;
    let first = placements[0];
    let focused = fixture.state.focus;
    assert_ne!(
        focused,
        Some(first.id),
        "the second window should hold focus"
    );

    // A click over the bar, in the first window's own column.
    fixture.state.pointer_move((first.rect.x + 5) as f64, 15.0);
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, true);
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    assert_eq!(
        fixture.state.focus, focused,
        "a click on the bar moved window focus"
    );

    // ...and the same click on the window below the bar does focus it.
    fixture
        .state
        .pointer_move((first.rect.x + 5) as f64, (first.rect.y + 25) as f64);
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, true);
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    assert_eq!(
        fixture.state.focus,
        Some(first.id),
        "a click on the window should have focused it"
    );
}

/// A click on a bar reaches the bar's own client: the `wl_pointer.enter`
/// the move establishes, then the press and the release -- the exact
/// `Request::Click` shape (move-then-press-release) the field report's
/// `scoot msg pointer click 30 15` takes. Asserted on what the client's
/// `wl_pointer` was actually sent, not on the compositor's hit test, which
/// is what was never pinned and what the report says is dead.
/// gh issue #182.
///
/// Closed as could-not-reproduce (live `--nested` probes on current `main`
/// *and* on the reported rev both deliver every click shape, and this test
/// passes unmodified): kept as the pin so the path stays green, in the
/// field report's own shapes -- a 30px exclusive-zone bar clicked left and
/// right, plus the launcher overlay the report names, plus the middle
/// button for completeness.
#[test]
fn a_click_reaches_the_bar_press_and_release() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 1,
        color: BAR_BGRA,
    });

    // The move half: the bar's client must be told the pointer entered its
    // surface, or the button below has nowhere to go.
    fixture.state.pointer_move(30.0, 15.0);
    fixture.settle();
    assert_eq!(
        fixture.pointer_focus(),
        Some(Focused::Layer(0)),
        "moving over the bar should enter the bar's surface (gh #182)"
    );

    // The button half, each edge separately, every button the report names
    // and the middle one for completeness: a client that only ever sees
    // presses, or only releases, still can't work its icons — so the press
    // and the release each get their own assertion rather than one
    // before/after compare across the pair.
    for button in [
        scoot_ipc::PointerButton::Left,
        scoot_ipc::PointerButton::Right,
        scoot_ipc::PointerButton::Middle,
    ] {
        let before = fixture.serials().button;
        fixture.state.pointer_button(button, true);
        fixture.settle();
        let pressed = fixture.serials().button;
        assert_ne!(
            before, pressed,
            "pressing {button:?} over the bar should send the bar's client a button press (gh #182)"
        );
        fixture.state.pointer_button(button, false);
        fixture.settle();
        let after = fixture.serials().button;
        assert_ne!(
            pressed, after,
            "releasing {button:?} over the bar should send the bar's client a button release (gh #182)"
        );
    }

    // ...and the launcher overlay the report names takes a click too.
    fixture.state.pointer_move(ON_LAUNCHER.0, ON_LAUNCHER.1);
    fixture.settle();
    assert_eq!(
        fixture.pointer_focus(),
        Some(Focused::Layer(1)),
        "moving over the launcher should enter the launcher's surface (gh #182)"
    );
    let before = fixture.serials().button;
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, true);
    fixture.settle();
    let pressed = fixture.serials().button;
    assert_ne!(
        before, pressed,
        "pressing on the launcher should send its client a button press (gh #182)"
    );
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    fixture.settle();
    assert_ne!(
        pressed,
        fixture.serials().button,
        "releasing on the launcher should send its client a button release (gh #182)"
    );
}

// -------------------------------------------------------------------------
// Keyboard interactivity
//
// Every assertion below is on what the *client's* `wl_keyboard` was told --
// `enter`, `leave`, `key` -- rather than on a compositor-side field. "Who
// holds keyboard focus" is a claim about what reached a client, and a test
// that reads `State::clicked_layer` would pass just as happily with nothing
// ever sent over the wire.
//
// The coordinates: the launcher is [`LayerSpec::launcher`]'s 60-square box in
// the bottom-right corner (140..200 on both axes), a window's own buffer is
// [`WINDOW_BUFFER`] square at (12, 12), and (100, 100) is bare desktop --
// no window, no layer surface, nothing.
// -------------------------------------------------------------------------

/// The headline: an `exclusive` surface on `overlay` takes the keyboard the
/// moment it maps, the window is told it lost it, and typed keys really
/// arrive at the layer surface.
///
/// This is the case a launcher and a layer-shell lock screen both need, and
/// the one that was actively dangerous before: the surface used to map and
/// draw over everything while every keystroke went to the window behind it.
#[test]
fn an_exclusive_overlay_surface_takes_the_keyboard_when_it_maps() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.keyboard();
    assert_eq!(
        before.focused,
        Some(Focused::Window(0)),
        "the window should start with the keyboard"
    );

    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "a layer surface with no buffer yet must not take the keyboard"
    );

    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    let mapped = fixture.keyboard();
    assert_eq!(
        mapped.focused,
        Some(Focused::Layer(0)),
        "an exclusive overlay surface should hold the keyboard once mapped"
    );
    assert_eq!(
        mapped.leaves,
        before.leaves + 1,
        "the window should have been told it lost the keyboard"
    );

    fixture.state.type_text("hi").expect("two typed characters");
    fixture.settle();
    let typed = fixture.keyboard();
    assert_eq!(
        typed.keys,
        mapped.keys + 4,
        "two characters, pressed and released, should have reached the launcher"
    );
    assert_eq!(typed.focused, Some(Focused::Layer(0)));
    // Window focus -- the ring, `set_activated`, `scoot msg windows` --
    // deliberately does not move: it tracks where focus returns to.
    assert!(fixture.state.focus.is_some(), "the window is still focused");
}

/// The other half of the same guarantee, and the one that must not regress:
/// a bar, a wallpaper or a notification daemon asks for `none`, and nothing
/// about the keyboard changes for it -- ever.
#[test]
fn a_bar_that_wants_no_keyboard_never_takes_it() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.keyboard();

    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    let after = fixture.keyboard();
    assert_eq!(after.focused, Some(Focused::Window(0)));
    assert_eq!(
        (after.enters, after.leaves),
        (before.enters, before.leaves),
        "a `none` bar mapping must not move keyboard focus at all"
    );

    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    let typed = fixture.keyboard();
    assert_eq!(typed.keys, before.keys + 2, "keys should reach the window");
    assert_eq!(typed.focused, Some(Focused::Window(0)));

    // ...and clicking it still doesn't, which is the pointer-side half.
    fixture.click(100.0, 15.0);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
}

/// A surface that commits but never attaches a buffer has nothing on screen,
/// so it cannot have the keyboard however loudly it asks -- otherwise any
/// client could swallow every keystroke while drawing nothing at all.
#[test]
fn a_layer_surface_with_no_buffer_cannot_hold_the_keyboard() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.keyboard();
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));

    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    let typed = fixture.keyboard();
    assert_eq!(typed.focused, Some(Focused::Window(0)));
    assert_eq!(
        typed.keys,
        before.keys + 2,
        "the keystroke should have gone to the window, not the empty surface"
    );
    assert!(!fixture.state.keyboard_on_layer);
}

/// `exclusive` on `background` is where the spec hands the decision back
/// ("for the bottom and background layers, the compositor is allowed to use
/// normal focus semantics"), and scoot's answer is click-to-focus: nothing
/// should be typing into a wallpaper by default.
#[test]
fn an_exclusive_background_surface_only_gets_the_keyboard_by_being_clicked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::wallpaper().with_keyboard(KeyboardInteractivity::Exclusive),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: WALLPAPER_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "a wallpaper must not take the keyboard just by asking"
    );

    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    let clicked = fixture.keyboard();
    assert_eq!(
        clicked.focused,
        Some(Focused::Layer(0)),
        "clicking it should focus it, like any other window"
    );
    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    assert_eq!(fixture.keyboard().keys, clicked.keys + 2);

    fixture.click(ON_WINDOW.0, ON_WINDOW.1);
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "clicking the window should take it back"
    );
}

/// `on_demand` is click-to-focus on any layer, and -- the half the spec is
/// explicit about -- the user has to be able to click *out* of it again.
#[test]
fn an_on_demand_surface_is_focused_and_unfocused_by_clicking() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "on_demand must wait to be clicked"
    );

    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    // Bare desktop: no window, no layer surface. Still a way out.
    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "clicking bare desktop should release an on_demand surface"
    );

    // ...and so is a window.
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    fixture.click(ON_WINDOW.0, ON_WINDOW.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
}

/// Clicking a bar that wants no keyboard is also a way out of an `on_demand`
/// surface: the click is a deliberate act somewhere else, and the surface
/// the user left should not keep the keys.
#[test]
fn clicking_a_bar_releases_an_on_demand_surface() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 1,
        color: BAR_BGRA,
    });

    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    fixture.click(100.0, 15.0);
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "a click on the bar should have released the launcher"
    );
}

/// The keyboard comes back when the surface holding it goes away -- the one
/// thing a launcher's whole lifecycle depends on.
#[test]
fn destroying_an_exclusive_surface_returns_the_keyboard_to_the_window() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::DestroyLayer { index: 0 });
    let after = fixture.keyboard();
    assert_eq!(after.focused, Some(Focused::Window(0)));
    assert!(!fixture.state.keyboard_on_layer);

    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    assert_eq!(
        fixture.keyboard().keys,
        after.keys + 2,
        "typing should reach the window again"
    );
}

/// ...and when it unmaps itself with a null buffer, which is how a
/// layer-shell client hides without destroying its surface. The surface is
/// still in the layer map at this point, so nothing but the buffer test in
/// `layer_focus` catches this.
#[test]
fn unmapping_an_exclusive_surface_returns_the_keyboard() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::UnmapLayer { index: 0 });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
    assert!(!fixture.state.keyboard_on_layer);
}

/// A surface that *stops* asking for the keyboard has to lose it, which is
/// the transition a gate on "does this surface want keys" would silently
/// miss -- there is nothing left asking, so nothing would trigger a refresh.
#[test]
fn a_surface_that_commits_none_gives_the_keyboard_back() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::SetLayerKeyboard {
        index: 0,
        keyboard: KeyboardInteractivity::None,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "a surface that commits `none` must give the keyboard back"
    );
}

/// A click is *spent*, not stored forever. An `on_demand` surface that gave
/// the keyboard back by committing `none` must not take it again the next
/// time it asks for `on_demand` -- there has been no new click, and the
/// focus ring, `set_activated` and `scoot msg windows` all still name the
/// window, so the keystrokes would go somewhere nothing on screen points at.
///
/// `none` <-> `on_demand` is the normal lifecycle for such a client (a bar
/// collapsing and re-opening a search field, a notification daemon closing
/// and re-opening an inline reply), not an edge case.
#[test]
fn an_on_demand_surface_does_not_recapture_the_keyboard_after_committing_none() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::SetLayerKeyboard {
        index: 0,
        keyboard: KeyboardInteractivity::None,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));

    fixture.run(Step::SetLayerKeyboard {
        index: 0,
        keyboard: KeyboardInteractivity::OnDemand,
    });
    let back = fixture.keyboard();
    assert_eq!(
        back.focused,
        Some(Focused::Window(0)),
        "asking for `on_demand` again must not re-focus the surface: the \
         click that focused it was spent when it committed `none`"
    );
    assert!(
        fixture.state.clicked_layer.is_none(),
        "the spent click should have been forgotten"
    );
    assert!(!fixture.state.keyboard_on_layer);

    // ...and the keys really follow, not just the `enter`.
    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    let typed = fixture.keyboard();
    assert_eq!(typed.keys, back.keys + 2, "typing should reach the window");
    assert_eq!(typed.focused, Some(Focused::Window(0)));

    // The surface is not broken, just no longer focused for free: a real
    // click still gives it the keyboard.
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
}

/// The same rule by the other route a client takes: unmapping with a null
/// buffer and showing itself again -- exactly what a launcher does when it
/// is dismissed and re-shown. `layer_focus` reads `Never` for the unmapped
/// surface, so the click is spent there too.
#[test]
fn an_on_demand_surface_does_not_recapture_the_keyboard_when_it_maps_again() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::UnmapLayer { index: 0 });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));

    fixture.run(Step::RemapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    let back = fixture.keyboard();
    assert_eq!(
        back.focused,
        Some(Focused::Window(0)),
        "mapping again must not re-focus the surface without a new click"
    );
    assert!(fixture.state.clicked_layer.is_none());
    assert!(!fixture.state.keyboard_on_layer);
    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    assert_eq!(fixture.keyboard().keys, back.keys + 2);

    // It really did map again -- its pixels are back in the corner, so the
    // assertion above is about focus, not about a surface that never
    // returned.
    let pixels = fixture.render();
    assert_pixel(
        &pixels,
        CANVAS - 1,
        CANVAS - 1,
        BAR_BGRA,
        "the re-mapped launcher",
    );
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
}

/// The transition that must *not* change: `exclusive` relaxing to
/// `on_demand` keeps the keyboard when the surface was clicked while it held
/// it. That path never passes through `Never`, so forgetting a spent click
/// must not touch it -- the click is what carries the focus over, which is
/// what `click_layer` records an `exclusive` surface's click for.
#[test]
fn an_exclusive_surface_that_relaxes_to_on_demand_keeps_a_click() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);

    fixture.run(Step::SetLayerKeyboard {
        index: 0,
        keyboard: KeyboardInteractivity::OnDemand,
    });
    let relaxed = fixture.keyboard();
    assert_eq!(
        relaxed.focused,
        Some(Focused::Layer(0)),
        "a clicked surface that relaxes to `on_demand` should keep the keyboard"
    );
    fixture.state.type_text("a").expect("a typed character");
    fixture.settle();
    assert_eq!(fixture.keyboard().keys, relaxed.keys + 2);

    // ...and it is genuinely `on_demand` now, not stuck: a click elsewhere
    // takes the keyboard back, which `exclusive` would have refused.
    fixture.click(ON_WINDOW.0, ON_WINDOW.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
}

/// The control for the test above: without a click there is nothing to carry
/// over, so the same relaxation gives the keyboard back to the window. This
/// is what makes that test a statement about the click rather than about
/// `on_demand` surfaces keeping focus in general.
#[test]
fn an_exclusive_surface_that_relaxes_to_on_demand_unclicked_gives_the_keyboard_back() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::SetLayerKeyboard {
        index: 0,
        keyboard: KeyboardInteractivity::OnDemand,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "nothing clicked it, so `on_demand` has no focus to hold on to"
    );
    assert!(fixture.state.clicked_layer.is_none());
}

/// Two surfaces both demanding exclusive focus: `overlay` beats `top`, the
/// same order everything else in this compositor stacks them in, and the
/// keyboard falls to the next one down when the winner goes away rather than
/// to the window.
#[test]
fn the_front_most_exclusive_surface_wins_and_focus_falls_back_when_it_goes() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).on_layer(zwlr_layer_shell_v1::Layer::Top),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 1,
        color: WALLPAPER_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(1)),
        "the overlay surface should win over the top one"
    );

    fixture.run(Step::DestroyLayer { index: 1 });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(0)),
        "the remaining exclusive surface should take the keyboard"
    );
    fixture.run(Step::DestroyLayer { index: 0 });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
}

/// The escape hatch, and the reason it is safe to let a full-screen client
/// take every keystroke: keybindings are matched before anything is
/// forwarded, so `Super+h` still works -- and so, on `--tty`, do the
/// `Ctrl+Alt+F<n>` VT switches that use the identical path.
#[test]
fn a_keybinding_still_fires_while_a_layer_surface_holds_the_keyboard() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    let before = fixture.keyboard();
    assert_eq!(before.focused, Some(Focused::Layer(0)));

    let first = fixture.state.world.arrange().placements[0].id;
    assert_ne!(
        fixture.state.focus,
        Some(first),
        "the second window should hold window focus"
    );
    fixture
        .state
        .press(&scoot_ipc::KeyCombo {
            key: "h".into(),
            modifiers: vec![scoot_ipc::Modifier::Super],
        })
        .expect("a pressable combo");
    fixture.settle();

    assert_eq!(
        fixture.state.focus,
        Some(first),
        "Super+h should still move window focus"
    );
    let after = fixture.keyboard();
    assert_eq!(
        after.focused,
        Some(Focused::Layer(0)),
        "the layer surface should still hold the keyboard"
    );
    assert_eq!(
        after.keys,
        before.keys + 2,
        "only the Super modifier's own press and release should have been \
         forwarded; the bound `h` must have been intercepted"
    );
}

/// No windows at all: the surface takes the keyboard, and when it goes the
/// keyboard goes nowhere rather than to a window that doesn't exist.
#[test]
fn an_exclusive_surface_with_no_windows_at_all_is_handled() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::DestroyLayer { index: 0 });
    let after = fixture.keyboard();
    assert_eq!(after.focused, None, "nothing left to hold the keyboard");
    assert!(!fixture.state.keyboard_on_layer);
    assert!(fixture.state.clicked_layer.is_none());
    // The frame after all that still renders.
    assert_eq!(fixture.render().len(), (CANVAS * CANVAS * 4) as usize);
}

/// A client that simply dies while its layer surface holds the keyboard --
/// the crash-mid-operation case. Nothing is left pointing at it, and the
/// compositor keeps serving.
#[test]
fn a_client_that_disconnects_while_holding_the_keyboard_leaves_nothing_behind() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    assert!(fixture.state.clicked_layer.is_some());

    fixture.disconnect_client();
    assert!(
        fixture.state.clicked_layer.is_none(),
        "a dead client's surface must not be held here"
    );
    assert!(!fixture.state.keyboard_on_layer);
    assert_eq!(fixture.usable(), WHOLE);
    assert_eq!(fixture.render().len(), (CANVAS * CANVAS * 4) as usize);
}
