//! The window-title module through the harness: views, focus, actions,
//! truncation and the flood cap, with the protocol side injected as plain
//! data (building real `wlr-foreign-toplevel-management-v1` objects needs
//! a compositor; the integration tests in `tests/window_title.rs` drive
//! those). Every test starts the module as the bar does, through
//! [`Harness::start`].

use std::time::Duration;

use ab_glyph::{FontArc, FontVec};

use super::{Link, Standing, TITLE_INTERVAL, Tl};
use crate::action::{Action, ModuleAction, Trigger};
use crate::density::Scale;
use crate::modules::harness::Harness;
use crate::modules::{ClickCtx, CustomDraw, Input, InvokeError, OutputView, Update, find};
use crate::paint::{Canvas, Span};
use crate::testfont;
use crate::text::Text;
use crate::theme::Theme;

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };
const DP2: OutputView<'static> = OutputView { name: Some("DP-2") };
const EM: f32 = 50.0;
const PAD: u32 = 8;

fn text() -> Text {
    let font = FontArc::new(FontVec::try_from_vec(testfont::build()).unwrap());
    Text::new(font)
}

/// The module as the bar starts it, with its shared state to inject
/// toplevels through.
fn started() -> (Harness, Link) {
    let spec = find("window-title").expect("the window-title module is built");
    let settings = super::super::Settings::default();
    let link = settings.window_title.link.clone();
    let harness = Harness::start(spec, &settings).expect("window-title starts anywhere");
    (harness, link)
}

fn started_with(settings: super::super::Settings) -> (Harness, Link) {
    let spec = find("window-title").expect("the window-title module is built");
    let link = settings.window_title.link.clone();
    let harness = Harness::start(spec, &settings).expect("window-title starts anywhere");
    (harness, link)
}

fn with(show_app_id: bool, placeholder: &str, allow_close: bool) -> super::super::Settings {
    let mut settings = super::super::Settings::default();
    settings.window_title.show_app_id = show_app_id;
    settings.window_title.placeholder = placeholder.to_owned();
    settings.window_title.allow_close = allow_close;
    settings
}

/// Shows a toplevel on already-borrowed state (for tests holding the
/// borrow across several mutations).
fn show_inner(
    shared: &mut super::Shared,
    title: &str,
    app_id: &str,
    activated: bool,
    outputs: &[&str],
) {
    assert!(shared.toplevels_len < super::MAX_TOPLEVELS);
    let index = shared.toplevels_len;
    let toplevel = &mut shared.toplevels[index];
    *toplevel = Tl {
        handle: None,
        ..Tl::default()
    };
    super::store(&mut toplevel.title, &mut toplevel.title_len, title);
    super::store(&mut toplevel.app_id, &mut toplevel.app_id_len, app_id);
    for name in outputs {
        let mut standing = Standing::default();
        standing.set_name(name);
        toplevel.outputs[toplevel.outputs_len] = standing;
        toplevel.outputs_len += 1;
    }
    toplevel.activated = activated;
    shared.toplevels_len += 1;
}

/// Shows one toplevel as a batch of events would: `outputs` are the names
/// it stands on, already resolved (as `output_enter` plus the `wl_output`
/// name would leave them).
fn show(link: &Link, title: &str, app_id: &str, activated: bool, outputs: &[&str]) {
    let mut shared = link.0.borrow_mut();
    show_inner(&mut shared, title, app_id, activated, outputs);
    shared.bump_focus();
}

#[test]
fn no_windows_is_an_empty_view_not_an_error() {
    let (harness, _) = started();
    assert_eq!(harness.source_count(), 0);
    assert!(harness.view().is_empty());
    assert!(harness.view_on(Some("DP-1")).is_empty());
    assert_eq!(harness.value_on(Some("DP-1")), None);
}

#[test]
fn each_output_shows_its_own_focused_title() {
    let (mut harness, link) = started();
    show(&link, "editor", "foot", true, &["DP-1"]);
    show(&link, "browser", "firefox", true, &["DP-2"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.dispatch(), Update::Unchanged);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "editor");
    assert_eq!(harness.view_on(Some("DP-2")).text(), "browser");
    assert!(harness.view_on(Some("DP-3")).is_empty());
    assert!(harness.view().is_empty());
}

#[test]
fn an_unfocused_window_shows_nothing() {
    let (mut harness, link) = started();
    show(&link, "background", "foot", false, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert!(harness.view_on(Some("DP-1")).is_empty());
    assert_eq!(harness.value_on(Some("DP-1")), None);
}

#[test]
fn a_window_on_no_known_output_shows_nothing() {
    let (mut harness, link) = started();
    show(&link, "nowhere", "foot", true, &[]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert!(harness.view_on(Some("DP-1")).is_empty());
}

#[test]
fn the_app_id_shows_when_configured() {
    let (mut harness, link) = started_with(with(true, "", false));
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "editor - foot");
    // Without it, the title alone.
    let (mut plain, link) = started_with(with(false, "", false));
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(plain.dispatch(), Update::Changed);
    assert_eq!(plain.view_on(Some("DP-1")).text(), "editor");
}

#[test]
fn an_empty_title_shows_the_app_id() {
    let (mut harness, link) = started();
    show(&link, "", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "foot");
}

#[test]
fn the_placeholder_shows_when_none_focused() {
    let (mut harness, link) = started_with(with(false, "—", false));
    assert_eq!(harness.dispatch(), Update::Unchanged);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "—");
    // ... but it is not a value: no window is shown.
    assert_eq!(harness.value_on(Some("DP-1")), None);
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "editor");
}

#[test]
fn the_value_is_the_title_and_app_id() {
    let (mut harness, link) = started();
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(
        harness.value_on(Some("DP-1")),
        Some(serde_json::json!({"title": "editor", "app_id": "foot", "fullscreen": false}))
    );
    assert_eq!(harness.value_on(Some("DP-2")), None);
}

#[test]
fn control_characters_are_stripped_before_the_view() {
    let (mut harness, link) = started();
    show(&link, "a\tb\nc\x1bd\x7f", "e\x00f", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    let view = harness.view_on(Some("DP-1"));
    assert_eq!(view.text(), "abcd");
    assert!(!view.text().chars().any(char::is_control));
    assert!(!view.tooltip().chars().any(char::is_control));
    // CJK passes through untouched.
    show(&link, "日本語タイトル", "", true, &["DP-2"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.view_on(Some("DP-2")).text(), "日本語タイトル");
}

#[test]
fn the_tooltip_carries_the_full_title() {
    let (mut harness, link) = started_with(with(true, "", false));
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.view_on(Some("DP-1")).tooltip(), "editor - foot");
}

/// A click context in the module's span: the hit test is the span itself
/// (the whole title activates).
fn input_of(harness: &Harness, output: OutputView<'_>, trigger: Trigger) -> Option<Action> {
    let view = harness.view_on(output.name);
    let font = text();
    let ctx = ClickCtx {
        output,
        x: 10,
        view: &view,
        text: &font,
        em: EM,
        padding: PAD,
        span_width: 400,
        height: 60,
        scale: Scale::Integer(1),
    };
    harness.input(&Input { trigger, at: &ctx })
}

#[test]
fn a_click_activates_and_a_middle_click_closes_only_when_allowed() {
    let (mut harness, link) = started_with(with(false, "", true));
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(
        input_of(&harness, DP1, Trigger::Click),
        Some(Action::Module(ModuleAction::new("activate", None)))
    );
    assert_eq!(
        input_of(&harness, DP1, Trigger::MiddleClick),
        Some(Action::Module(ModuleAction::new("close", None)))
    );
    // No window here: nothing armed, so a press on the placeholder warns
    // nothing.
    assert_eq!(input_of(&harness, DP2, Trigger::Click), None);
    assert_eq!(input_of(&harness, DP2, Trigger::MiddleClick), None);
    // Scrolls and right clicks have no default.
    assert_eq!(input_of(&harness, DP1, Trigger::ScrollUp), None);
    assert_eq!(input_of(&harness, DP1, Trigger::RightClick), None);

    let (mut closed, link) = started_with(with(false, "", false));
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(closed.dispatch(), Update::Changed);
    assert_eq!(
        input_of(&closed, DP1, Trigger::Click),
        Some(Action::Module(ModuleAction::new("activate", None)))
    );
    assert_eq!(input_of(&closed, DP1, Trigger::MiddleClick), None);
    let _ = link;
}

#[test]
fn invoke_activates_and_closes_with_the_same_gates() {
    let (mut harness, link) = started_with(with(false, "", true));
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    // No handle to send on in a unit test (and no seat): activating a
    // live manager's window without one is refused by name.
    link.0.borrow_mut().live = true;
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("activate", None), 1),
        Err(InvokeError::Refused("no seat to activate on"))
    );
    // A close needs no seat: with no handle it sends nothing.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("close", None), 1),
        Ok(Update::Unchanged)
    );
    // Nowhere shown: refused, not silent.
    assert_eq!(
        harness.invoke(&DP2, &ModuleAction::new("activate", None), 1),
        Err(InvokeError::Refused("no focused window on this output"))
    );
    // A dead manager sends nothing (no protocol error), whatever is shown.
    link.0.borrow_mut().live = false;
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("close", None), 1),
        Ok(Update::Unchanged)
    );
    // Bad names and numbers are errors whatever the compositor is doing.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("raise", None), 1),
        Err(InvokeError::Unknown)
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("activate", Some(3)), 1),
        Err(InvokeError::NoArg)
    );
}

#[test]
fn invoke_close_needs_allow_close() {
    let (mut harness, link) = started_with(with(false, "", false));
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    link.0.borrow_mut().live = true;
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("close", None), 1),
        Err(InvokeError::Refused(
            "closing is off: set window-title.allow-close = true"
        ))
    );
}

#[test]
fn a_title_flood_is_one_redraw_then_held_for_the_timer() {
    let (mut harness, link) = started();
    // The first title draws at once.
    show(&link, "0%", "xterm", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "0%");
    // A hundred retitles before the next turn: one dispatch, and the
    // second draw is held for the flush timer.
    {
        let mut shared = link.0.borrow_mut();
        for n in 1..100 {
            retitle(&mut shared, 0, &format!("{n}%"));
        }
    }
    assert_eq!(harness.dispatch(), Update::Unchanged);
    // The state moved but nothing was drawn: the view reads live state
    // (what `query` answers), while the redraw waits for the timer.
    assert_eq!(harness.view_on(Some("DP-1")).text(), "99%");
    assert_eq!(harness.source_count(), 1);
    // The timer fires: the redraw is reported, once for the hundred titles.
    assert_eq!(
        harness.deliver(0, rustix::event::PollFlags::IN),
        Update::Changed
    );
    assert_eq!(harness.view_on(Some("DP-1")).text(), "99%");
    assert_eq!(harness.source_count(), 0);
    assert_eq!(harness.dispatch(), Update::Unchanged);
}

#[test]
fn held_titles_refresh_on_a_fixed_period() {
    let (mut harness, link) = started();
    show(&link, "a", "xterm", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    // Held for the flush timer ...
    {
        let mut shared = link.0.borrow_mut();
        retitle(&mut shared, 0, "b");
    }
    assert_eq!(harness.dispatch(), Update::Unchanged);
    assert_eq!(harness.source_count(), 1);
    // ... and a second title near the deadline does not move it: the
    // flush still fires on the first period, about 100 ms after the hold
    // began, rather than 100 ms after this title (which would freeze a
    // gapless flood until it pauses). Margins: the sleep must land inside
    // the first period (20 ms of room past it), and the wait must end
    // before a re-armed deadline would fire (40 ms of room before it); a
    // timerfd cannot fire early, and the wait's own timeout is
    // kernel-timed, so only a 20 ms overshoot of the sleep itself —
    // microseconds of work stand between them — breaks the first side.
    std::thread::sleep(Duration::from_millis(80));
    {
        let mut shared = link.0.borrow_mut();
        retitle(&mut shared, 0, "c");
    }
    assert_eq!(harness.dispatch(), Update::Unchanged);
    // Within one more period the held titles draw, once, as the latest.
    assert_eq!(
        harness.wait(Duration::from_millis(60)),
        Some(Update::Changed)
    );
    assert_eq!(harness.view_on(Some("DP-1")).text(), "c");
    assert_eq!(harness.source_count(), 0);
    assert_eq!(harness.dispatch(), Update::Unchanged);
}

#[test]
fn focus_moves_draw_at_once_even_mid_flood() {
    let (mut harness, link) = started();
    show(&link, "0%", "xterm", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    // Hold back a title ...
    {
        let mut shared = link.0.borrow_mut();
        retitle(&mut shared, 0, "1%");
    }
    assert_eq!(harness.dispatch(), Update::Unchanged);
    // ... then move focus: at once, not at the timer.
    {
        let mut shared = link.0.borrow_mut();
        shared.toplevels[0].activated = false;
        show_inner(&mut shared, "other", "foot", true, &["DP-1"]);
        shared.bump_focus();
    }
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "other");
    // The held title was consumed with the focus: nothing armed.
    assert_eq!(harness.source_count(), 0);
}

/// Retitles toplevel `index` as a `title` event would.
fn retitle(shared: &mut super::Shared, index: usize, title: &str) {
    let toplevel = &mut shared.toplevels[index];
    super::store(&mut toplevel.title, &mut toplevel.title_len, title);
    shared.bump();
}

#[test]
fn titles_wait_out_the_interval_then_draw() {
    let (mut harness, link) = started();
    show(&link, "first", "xterm", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    std::thread::sleep(TITLE_INTERVAL + Duration::from_millis(20));
    {
        let mut shared = link.0.borrow_mut();
        retitle(&mut shared, 0, "second");
    }
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "second");
    assert_eq!(harness.source_count(), 0);
}

#[test]
fn a_closed_window_is_gone_at_once() {
    let (mut harness, link) = started();
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    {
        let mut shared = link.0.borrow_mut();
        shared.toplevels_len = 0;
        shared.bump_focus();
    }
    assert_eq!(harness.dispatch(), Update::Changed);
    assert!(harness.view_on(Some("DP-1")).is_empty());
}

#[test]
fn long_titles_are_kept_whole_in_state_but_bounded() {
    let (mut harness, link) = started();
    let long = "x".repeat(10_000);
    show(&link, &long, "", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    let view = harness.view_on(Some("DP-1"));
    assert!(view.text().len() <= super::super::MAX_TEXT);
}

#[test]
fn a_hovered_title_is_the_hover_token_not_the_accent() {
    use super::super::{Init, View};
    let settings = super::super::Settings::default();
    let link = settings.window_title.link.clone();
    show(&link, "app 12", "", true, &["DP-1"]);
    // The module itself, on the same shared state: what the bar draws.
    let module = match super::init(&settings) {
        Init::Available(module) => module,
        Init::Unavailable(why) => panic!("window-title unavailable: {why}"),
    };
    let mut view = View::default();
    module.view(&DP1, &mut view);
    let theme = Theme {
        hover: crate::color::Color {
            r: 0x01,
            g: 0x02,
            b: 0x03,
        },
        ..Theme::default()
    };
    let mut font = text();
    let baseline = font.metrics(EM).baseline(60);
    let mut pixels = vec![0u8; 60 * 400 * 4];
    let mut canvas = Canvas::new(&mut pixels, 400, 60).unwrap();
    // Narrow enough to truncate: the module draws itself, hovered.
    let mut custom = CustomDraw {
        output: DP1,
        view: &view,
        canvas: &mut canvas,
        text: &mut font,
        span: Span { x: 0, width: 100 },
        em: EM,
        baseline,
        padding: PAD,
        hovered: true,
        scale: Scale::Integer(1),
        theme: &theme,
    };
    assert!(module.custom_draw(&mut custom));
    // Title ink in the hover token, which differs from the accent here.
    assert!(
        pixels
            .chunks_exact(4)
            .any(|p| [p[2], p[1], p[0]] == [0x01, 0x02, 0x03]),
        "no hover-token ink in a hovered truncated title"
    );
}

#[test]
fn truncation_cuts_with_an_ellipsis_by_pixels_not_chars() {
    use super::super::{Init, View};
    let settings = super::super::Settings::default();
    let link = settings.window_title.link.clone();
    show(&link, "abcdefghijklmnopqrstuvwxyz", "", true, &["DP-1"]);
    // The module itself, on the same shared state: what the bar draws.
    let module = match super::init(&settings) {
        Init::Available(module) => module,
        Init::Unavailable(why) => panic!("window-title unavailable: {why}"),
    };
    let mut view = View::default();
    module.view(&DP1, &mut view);
    let mut font = text();
    let theme = Theme::default();
    let full = font.measure(None, view.text(), EM);
    assert!(full > 0);
    let baseline = font.metrics(EM).baseline(60);
    let mut draw = |span: Span| {
        let mut pixels = vec![0u8; 60 * 400 * 4];
        let mut canvas = Canvas::new(&mut pixels, 400, 60).unwrap();
        let mut custom = CustomDraw {
            output: DP1,
            view: &view,
            canvas: &mut canvas,
            text: &mut font,
            span,
            em: EM,
            baseline,
            padding: PAD,
            hovered: false,
            scale: Scale::Integer(1),
            theme: &theme,
        };
        module.custom_draw(&mut custom)
    };
    // A span exactly the text wide: fits, so the plain draw stands.
    assert!(!draw(Span {
        x: 0,
        width: full + PAD * 2
    }));
    // A narrower span: cut with an ellipsis, still inside the span.
    assert!(draw(Span {
        x: 0,
        width: full / 2
    }));
    // A span too narrow for even the ellipsis: blank, not a panic.
    assert!(draw(Span { x: 0, width: 1 }));
}

#[test]
fn a_static_icon_stands_before_the_title() {
    use crate::icon::Icon;
    let mut settings = with(false, "", false);
    settings.window_title.icon = Some(Icon::Glyph('W'));
    let (mut harness, link) = started_with(settings);
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    let view = harness.view_on(Some("DP-1"));
    assert_eq!(view.text(), "editor");
    assert_eq!(view.icon(), Some('W'));
}

#[test]
fn the_placeholder_gets_no_icon() {
    use crate::icon::Icon;
    // A placeholder is not a window: the text shows, the icon does not.
    let mut settings = with(false, "empty", false);
    settings.window_title.icon = Some(Icon::Glyph('W'));
    let (harness, _link) = started_with(settings);
    let view = harness.view_on(Some("DP-1"));
    assert_eq!(view.text(), "empty");
    assert!(view.icon().is_none() && view.art().is_none());
}

#[test]
fn icon_only_draws_the_icon_with_the_title_in_the_tooltip() {
    use crate::icon::Icon;
    let mut settings = with(false, "", false);
    settings.window_title.icon = Some(Icon::Glyph('W'));
    settings.window_title.show_text = false;
    let (mut harness, link) = started_with(settings);
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    let view = harness.view_on(Some("DP-1"));
    assert_eq!(view.text(), "");
    assert_eq!(view.icon(), Some('W'));
    // The tooltip already carries the full title.
    assert_eq!(view.tooltip(), "editor");
}

#[test]
fn icon_only_hides_without_a_window() {
    use crate::icon::Icon;
    // No window, an empty placeholder and `show-text = false`: the icon
    // stands for a window, so with none the module hides, as with an
    // empty placeholder.
    let mut settings = with(false, "", false);
    settings.window_title.icon = Some(Icon::Glyph('W'));
    settings.window_title.show_text = false;
    let (harness, _link) = started_with(settings);
    assert!(harness.view_on(Some("DP-1")).is_empty());
}

#[test]
fn without_an_icon_the_title_shows_text_alone() {
    let (mut harness, link) = started();
    show(&link, "editor", "foot", true, &["DP-1"]);
    assert_eq!(harness.dispatch(), Update::Changed);
    let view = harness.view_on(Some("DP-1"));
    assert_eq!(view.text(), "editor");
    assert!(view.icon().is_none() && view.art().is_none());
}
