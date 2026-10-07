//! The output model against every event ordering the glue can deliver.

use super::{Effect, Info, OutputId, Outputs, Size, Surface, Transform};
use crate::density::Scale;

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

/// One output bound from global 7, its initial burst received and settled,
/// as the glue sees it on a well-behaved v4 compositor.
fn settled_1080p(outputs: &mut Outputs<()>) -> OutputId {
    let id = outputs.add(7, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    output.stage_mode(true, 1920, 1080);
    output.stage_scale(1);
    output.stage_name("DP-1".into());
    output.stage_description("A monitor".into());
    output.done();
    assert_eq!(output.settled(), Effect::Create);
    id
}

#[test]
fn properties_apply_at_done_not_before() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(1, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    output.stage_name("HDMI-A-1".into());
    output.stage_mode(true, 2560, 1440);
    output.stage_scale(2);
    assert_eq!(output.info(), &Info::default(), "nothing before done");
    assert!(!output.has_seen_done());
    output.done();
    assert!(output.has_seen_done());
    assert_eq!(output.info().name.as_deref(), Some("HDMI-A-1"));
    assert_eq!(output.info().mode, Some(size(2560, 1440)));
    assert_eq!(output.info().scale, 2);
    assert_eq!(
        output.info().logical(Scale::Integer(output.info().scale)),
        Some(size(1280, 720))
    );
    // A later batch replaces only what it carries.
    output.stage_scale(1);
    assert_eq!(output.info().scale, 2);
    output.done();
    assert_eq!(output.info().scale, 1);
    assert_eq!(output.info().name.as_deref(), Some("HDMI-A-1"));
}

#[test]
fn the_surface_is_created_once_settled_and_only_once() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(1, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    output.done();
    assert_eq!(
        output.surface(),
        &Surface::Waiting,
        "done alone is not settled"
    );
    assert_eq!(output.settled(), Effect::Create);
    assert_eq!(output.surface(), &Surface::Pending);
    assert_eq!(output.settled(), Effect::None);
    output.done();
    assert_eq!(output.surface(), &Surface::Pending);
}

/// A v1 `wl_output` never sends `done`; a broken compositor might not
/// either. Settling applies what was staged rather than wait forever.
#[test]
fn an_output_that_never_sends_done_is_settled_with_what_it_sent() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(1, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    output.stage_mode(true, 800, 600);
    assert_eq!(output.settled(), Effect::Create);
    assert!(!output.has_seen_done());
    assert_eq!(output.info().mode, Some(size(800, 600)));
    assert_eq!(output.info().name, None);
}

#[test]
fn configure_is_acked_and_its_size_used() {
    let mut outputs = Outputs::<()>::default();
    let id = settled_1080p(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert_eq!(output.surface_size(), None, "pending has no size");
    assert_eq!(output.configure(5, 1920, 1080), Effect::Ack(5));
    assert_eq!(
        output.surface(),
        &Surface::Configured {
            serial: 5,
            requested: size(1920, 1080),
            drawn: None,
        }
    );
    assert_eq!(output.surface_size(), Some(size(1920, 1080)));
    // A new configure (the output changed mode) replaces it.
    assert_eq!(output.configure(9, 1280, 1024), Effect::Ack(9));
    assert_eq!(output.surface_size(), Some(size(1280, 1024)));
}

#[test]
fn a_zero_configure_takes_the_outputs_logical_size() {
    let mut outputs = Outputs::<()>::default();
    let id = settled_1080p(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert_eq!(output.configure(1, 0, 0), Effect::Ack(1));
    assert_eq!(output.surface_size(), Some(size(1920, 1080)));
    // Per axis.
    assert_eq!(output.configure(2, 0, 500), Effect::Ack(2));
    assert_eq!(output.surface_size(), Some(size(1920, 500)));
    assert_eq!(output.configure(3, 700, 0), Effect::Ack(3));
    assert_eq!(output.surface_size(), Some(size(700, 1080)));
}

/// A 0×0 configure before the output has reported a mode: acked, but with
/// no size to draw at until the mode arrives, which then resolves it.
#[test]
fn a_zero_configure_before_the_mode_is_known_waits_for_it() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(1, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert_eq!(output.settled(), Effect::Create);
    assert_eq!(output.configure(1, 0, 0), Effect::Ack(1));
    assert_eq!(output.surface().name(), "configured");
    assert_eq!(output.surface_size(), None);
    output.stage_mode(true, 1024, 768);
    assert_eq!(output.surface_size(), None, "staged is not current");
    output.done();
    assert_eq!(output.surface_size(), Some(size(1024, 768)));
}

#[test]
fn scale_and_transform_changes_move_the_fallback_size() {
    let mut outputs = Outputs::<()>::default();
    let id = settled_1080p(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(1, 0, 0);
    output.stage_scale(2);
    output.done();
    assert_eq!(output.surface_size(), Some(size(960, 540)));
    output.stage_transform(Transform::Rotate90);
    output.done();
    assert_eq!(output.surface_size(), Some(size(540, 960)));
    output.stage_transform(Transform::Flipped180);
    output.stage_scale(3);
    output.done();
    // 1920/3 = 640, 1080/3 = 360: rounded down.
    assert_eq!(output.surface_size(), Some(size(640, 360)));
    // A configured, non-zero size is never overridden.
    let _ = output.configure(2, 100, 100);
    output.stage_scale(1);
    output.done();
    assert_eq!(output.surface_size(), Some(size(100, 100)));
}

#[test]
fn logical_size_rounds_down_at_odd_sizes_and_prefers_xdg_output() {
    let mut info = Info {
        mode: Some(size(1367, 769)),
        scale: 2,
        ..Info::default()
    };
    assert_eq!(info.logical(Scale::Integer(2)), Some(size(683, 384)));
    // The compositor's own logical size (fractional scales) wins.
    info.xdg_logical = Some(size(911, 513));
    assert_eq!(info.logical(Scale::Integer(2)), Some(size(911, 513)));
    assert_eq!(
        Info::default().logical(Scale::Integer(1)),
        None,
        "no mode, no size"
    );
}

#[test]
fn nonsense_from_the_compositor_is_ignored() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(1, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    output.stage_mode(true, 0, 1080);
    output.stage_mode(true, -1920, 1080);
    output.stage_mode(true, 1920, i32::MIN);
    output.stage_mode(false, 640, 480); // not the current mode
    output.stage_scale(0);
    output.stage_scale(-2);
    output.stage_xdg_logical(0, 0);
    output.stage_xdg_logical(-5, 10);
    output.done();
    assert_eq!(output.info(), &Info::default());
    // Huge but positive values are kept as they are: bounding a buffer
    // size is the buffer's job (scootbg-mem's `ShmBuffer`), and dividing
    // cannot overflow.
    output.stage_mode(true, i32::MAX, i32::MAX);
    output.stage_scale(i32::MAX);
    output.done();
    assert_eq!(
        output.info().logical(Scale::Integer(output.info().scale)),
        Some(size(1, 1))
    );
}

#[test]
fn closed_is_retried_once_then_given_up() {
    let mut outputs = Outputs::<()>::default();
    let id = settled_1080p(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(1, 1920, 1080);
    assert_eq!(output.closed(), Effect::DestroyAndRetry);
    assert_eq!(output.surface(), &Surface::Closed);
    assert_eq!(output.surface_size(), None);
    // A second `closed` for the destroyed surface is stale: nothing.
    assert_eq!(output.closed(), Effect::None);
    assert_eq!(output.configure(2, 10, 10), Effect::None, "stale configure");
    assert_eq!(output.retry(), Effect::Create);
    assert_eq!(output.surface(), &Surface::Pending);
    assert_eq!(output.retry(), Effect::None, "one retry per close");

    // Closed again, before or after a configure: given up.
    assert_eq!(output.closed(), Effect::DestroyAndGiveUp);
    assert_eq!(output.surface(), &Surface::GaveUp);
    assert_eq!(output.surface().name(), "gave-up");
    assert_eq!(output.retry(), Effect::None);
    assert_eq!(output.closed(), Effect::None);
    assert_eq!(output.configure(3, 10, 10), Effect::None);
    assert_eq!(output.settled(), Effect::None);
    // Properties still track the output.
    output.stage_scale(2);
    output.done();
    assert_eq!(output.info().scale, 2);
}

#[test]
fn closed_while_pending_counts_too() {
    let mut outputs = Outputs::<()>::default();
    let id = settled_1080p(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert_eq!(output.closed(), Effect::DestroyAndRetry);
    assert_eq!(output.retry(), Effect::Create);
    assert_eq!(output.closed(), Effect::DestroyAndGiveUp);
}

#[test]
fn giving_up_on_one_output_leaves_the_others_alone() {
    let mut outputs = Outputs::<()>::default();
    let a = settled_1080p(&mut outputs);
    let b = outputs.add(8, |_| ());
    assert_eq!(outputs.get_mut(b).unwrap().output.settled(), Effect::Create);
    let first = &mut outputs.get_mut(a).unwrap().output;
    let _ = first.closed();
    let _ = first.retry();
    let _ = first.closed();
    assert_eq!(first.surface(), &Surface::GaveUp);
    let second = &mut outputs.get_mut(b).unwrap().output;
    assert_eq!(second.surface(), &Surface::Pending);
    assert_eq!(second.configure(1, 5, 5), Effect::Ack(1));
}

/// Removal at every stage: the entry goes, and a callback or event still
/// in flight for it finds nothing (the glue drops it).
#[test]
fn removal_at_any_stage_leaves_nothing_behind() {
    type Step = fn(&mut super::Output);
    let stages: [(&str, Step); 6] = [
        ("bound, before done", |_| {}),
        ("done, not settled", |o| o.done()),
        ("pending", |o| {
            let _ = o.settled();
        }),
        ("configured", |o| {
            let _ = o.settled();
            let _ = o.configure(1, 10, 10);
        }),
        ("closed, retry in flight", |o| {
            let _ = o.settled();
            let _ = o.closed();
        }),
        ("gave up", |o| {
            let _ = o.settled();
            let _ = o.closed();
            let _ = o.retry();
            let _ = o.closed();
        }),
    ];
    for (what, step) in stages {
        let mut outputs = Outputs::<&str>::default();
        let keep = outputs.add(1, |_| "keep");
        let id = outputs.add(2, |_| "gone");
        step(&mut outputs.get_mut(id).unwrap().output);
        let removed = outputs.remove_global(2).expect(what);
        assert_eq!(removed.objects, "gone", "{what}");
        assert_eq!(removed.output.id(), id);
        assert!(outputs.get_mut(id).is_none(), "{what}: still found");
        assert!(outputs.remove_global(2).is_none(), "{what}: removed twice");
        assert_eq!(outputs.len(), 1);
        assert!(outputs.get_mut(keep).is_some());
    }
}

/// A replugged monitor may come back under its old registry name, and
/// always with its old connector name. Ids never repeat, so a callback
/// meant for the old one cannot reach the new one.
#[test]
fn ids_are_never_reused_even_when_names_are() {
    let mut outputs = Outputs::<()>::default();
    let old = outputs.add(4, |_| ());
    let _ = outputs.remove_global(4);
    let new = outputs.add(4, |_| ());
    assert_ne!(old, new);
    assert!(
        outputs.get_mut(old).is_none(),
        "a late result for the old one"
    );
    assert!(outputs.get_mut(new).is_some());
}

/// Two outputs with the same name (a compositor bug, or a replug whose new
/// global arrives before the old one's removal): both tracked, each by its
/// own global, and removing one leaves the other.
#[test]
fn duplicate_names_are_tracked_separately() {
    let mut outputs = Outputs::<()>::default();
    let a = outputs.add(1, |_| ());
    let b = outputs.add(2, |_| ());
    for id in [a, b] {
        let output = &mut outputs.get_mut(id).unwrap().output;
        output.stage_name("DP-1".into());
        output.done();
        assert_eq!(output.settled(), Effect::Create);
    }
    assert_eq!(outputs.len(), 2);
    let _ = outputs.remove_global(1);
    assert_eq!(outputs.len(), 1);
    let left = outputs.iter().next().unwrap();
    assert_eq!(left.output.id(), b);
    assert_eq!(left.output.info().name.as_deref(), Some("DP-1"));
}

#[test]
fn outputs_are_listed_in_the_order_they_appeared() {
    let mut outputs = Outputs::<u32>::default();
    for global in [10, 11, 12, 13] {
        outputs.add(global, |_| global);
    }
    let _ = outputs.remove_global(11);
    let order: Vec<u32> = outputs.iter().map(|e| e.objects).collect();
    assert_eq!(order, [10, 12, 13]);
}

#[test]
fn transforms_that_rotate_by_a_quarter_swap_axes() {
    use Transform::*;
    for t in [Rotate90, Rotate270, Flipped90, Flipped270] {
        assert!(t.swaps_axes(), "{t:?}");
    }
    for t in [Normal, Rotate180, Flipped, Flipped180] {
        assert!(!t.swaps_axes(), "{t:?}");
    }
}

/// At a fractional scale `wl_output` reports the scale rounded up, so the
/// derived logical size is too small (1600×1000 at 1.5 is 1067×667, not
/// 800×500). Once the compositor configures the surface, its size, which is
/// the output's, is what `logical` reports.
#[test]
fn the_configured_size_is_the_best_logical_size() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(1, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    output.stage_mode(true, 1600, 1000);
    output.stage_scale(2);
    output.done();
    let _ = output.settled();
    assert_eq!(output.logical(), Some(size(800, 500)), "the estimate");
    let _ = output.configure(1, 1067, 667);
    assert_eq!(output.logical(), Some(size(1067, 667)));
    assert_eq!(
        output.info().logical(Scale::Integer(output.info().scale)),
        Some(size(800, 500))
    );
    // Closed: back to the estimate until configured again.
    let _ = output.closed();
    assert_eq!(output.logical(), Some(size(800, 500)));
}

/// Output names come from the compositor and reach stderr: control
/// characters (a terminal escape, a newline forging a second log line)
/// are escaped.
#[test]
fn labels_escape_what_the_compositor_named_the_output() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(1, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert_eq!(output.label().to_string(), "an unnamed output");
    output.stage_name("DP-1".into());
    output.done();
    assert_eq!(output.label().to_string(), "output \"DP-1\"");
    output.stage_name("x\u{1b}[2J\nscootbg: fake \"line\"\\".into());
    output.done();
    assert_eq!(
        output.label().to_string(),
        "output \"x\\u{1b}[2J\\nscootbg: fake \\\"line\\\"\\\\\""
    );
}

/// `coming` is true exactly while a round trip already sent will say what
/// to draw the surface at, so an image `set` held for it is held for at
/// most that long.
#[test]
fn an_output_is_coming_until_its_configure_has_settled() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(7, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert!(output.coming(), "not settled yet");
    output.stage_name("DP-1".into());
    output.done();
    assert_eq!(output.settled(), Effect::Create);
    let creation = output.creation();
    assert!(output.coming(), "pending: its configure is on the way");
    assert_eq!(output.configure(5, 1920, 1080), Effect::Ack(5));
    assert!(output.coming(), "configured, its round trip still out");
    // A stale round trip (an older serial) settles nothing.
    output.configure_settled(4);
    assert!(output.coming());
    output.configure_settled(5);
    assert!(!output.coming(), "settled: draw for it now");
    // A new configure makes it coming again, until its own round trip.
    assert_eq!(output.configure(6, 1280, 720), Effect::Ack(6));
    assert!(output.coming());
    output.configure_settled(5);
    assert!(
        output.coming(),
        "the old serial's round trip does not count"
    );
    output.configure_settled(6);
    assert!(!output.coming());
    // Closed: re-created after a round trip, then pending again.
    assert_eq!(output.closed(), Effect::DestroyAndRetry);
    assert!(output.coming());
    assert_eq!(output.retry(), Effect::Create);
    assert!(output.coming());
    // A compositor late with the configure holds nothing up any longer.
    let late = output.creation();
    assert_ne!(late, creation);
    assert!(output.unanswered(late));
    assert!(!output.coming(), "late: no longer waited for");
    // Configured after all: coming until that settles.
    assert_eq!(output.configure(9, 1920, 1080), Effect::Ack(9));
    assert!(output.coming());
    output.configure_settled(9);
    assert!(!output.coming());
    // Closed a second time: given up, nothing to wait for.
    assert_eq!(output.closed(), Effect::DestroyAndGiveUp);
    assert!(!output.coming());
}

/// A surface made again (a `clear`) waits for its own configure: a serial
/// settled on the old surface does not carry over.
#[test]
fn a_recreated_surface_is_coming_again() {
    let mut outputs = Outputs::<()>::default();
    let id = settled_1080p(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(3, 1920, 1080);
    output.configure_settled(3);
    assert!(!output.coming());
    output.recreated();
    assert!(output.coming());
    let _ = output.configure(3, 1920, 1080);
    assert!(output.coming(), "the same serial on a new surface is new");
}

#[test]
fn a_requested_transition_is_consumed_once_at_its_generation() {
    use crate::transition::{Kind, Spec};
    let mut outputs = Outputs::<()>::default();
    let id = settled_1080p(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert_eq!(output.take_transition(), None);
    assert_eq!(output.running(), None);
    let spec = Spec {
        kind: Kind::Fade,
        duration_ms: 300,
        easing: crate::transition::Easing::Linear,
        angle_deg: 0.0,
        pos: (0.5, 0.5),
    };
    // Before any request the stamp is 0: a request at generation 1 waits
    // for its stamp, surviving takes until then.
    output.request_transition(spec, 1);
    assert_eq!(output.take_transition(), None, "stale: stamp is still 0");
    output.want(1);
    assert_eq!(output.take_transition(), Some(spec));
    assert_eq!(output.take_transition(), None, "consumed once");
    // A superseded request's transition waits too, until a newer request
    // replaces it.
    output.request_transition(spec, 2);
    assert_eq!(output.take_transition(), None, "stamp is still 1");
    output.want(2);
    assert_eq!(output.take_transition(), Some(spec));
    // The marker is independent bookkeeping for `query`.
    output.set_running(Some(Kind::Wipe));
    assert_eq!(output.running(), Some(Kind::Wipe));
    output.set_running(None);
    assert_eq!(output.running(), None);
}
