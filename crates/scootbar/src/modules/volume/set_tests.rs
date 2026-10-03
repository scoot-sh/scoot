//! The absolute `set` action and the slider popup, against the same fake
//! server as the rest (`fake.rs`).

use super::fake::*;
use super::proto::{self, Bounded, Kind};
use crate::action::ModuleAction;
use crate::modules::{InvokeError, OutputView, Update};

/// The volume the next `SET_SINK_VOLUME` carries, acked.
fn sent_volume(conn: &mut Conn) -> u32 {
    let (tag, payload) = conn.expect(proto::CMD_SET_SINK_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    let _ = reader.get_u32();
    let mut name = Bounded::empty();
    let _ = reader.get_str(&mut name);
    let mut vols = [0; proto::MAX_CHANNELS];
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[0], vols[1]);
    conn.ack(tag);
    vols[0]
}

#[test]
fn set_is_an_absolute_percent_and_needs_one() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("set", Some(80)), 1),
        Ok(Update::Unchanged)
    );
    assert_eq!(sent_volume(&mut conn), proto::from_percent(80));
    let _ = wait(&mut harness);
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Sink,
        SINK,
        &[proto::from_percent(80); 2],
        false,
    );
    assert_eq!(view_text(&harness), "80%");
    // Already there: nothing is sent.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("set", Some(80)), 1),
        Ok(Update::Unchanged)
    );
    conn.quiet(200);
    // No number is refused, with nothing sent.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("set", None), 1),
        Err(InvokeError::NeedsArg)
    );
    conn.quiet(100);
}

#[test]
fn set_is_held_to_zero_and_to_max_volume() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    let output = OutputView { name: None };
    // Past full scale on the default `max-volume` of 100: full scale.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("set", Some(500)), 1),
        Ok(Update::Unchanged)
    );
    assert_eq!(sent_volume(&mut conn), proto::NORM);
    let _ = wait(&mut harness);
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Sink,
        SINK,
        &[proto::NORM; 2],
        false,
    );
    // Below zero: zero, however negative.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("set", Some(i32::MIN)), 1),
        Ok(Update::Unchanged)
    );
    assert_eq!(sent_volume(&mut conn), 0);
}

#[test]
fn set_with_no_server_is_refused_not_queued() {
    let mut harness = sink_module(std::path::PathBuf::from("/nonexistent"));
    let output = OutputView { name: None };
    assert!(matches!(
        harness.invoke(&output, &ModuleAction::new("set", Some(10)), 1),
        Err(InvokeError::Refused(_))
    ));
}

#[cfg(feature = "popup")]
mod popup {
    use super::*;
    use crate::popup::{Content, Kind as Widget};

    fn content_of(harness: &mut crate::modules::harness::Harness) -> (bool, Content) {
        let mut content = Content::default();
        let shown = harness.popup(&mut content);
        (shown, content)
    }

    #[test]
    fn there_is_no_popup_while_there_is_no_device() {
        let mut harness = sink_module(std::path::PathBuf::from("/nonexistent"));
        let (shown, content) = content_of(&mut harness);
        assert!(!shown);
        assert!(content.is_empty());
    }

    #[test]
    fn the_popup_is_the_device_a_slider_at_its_level_and_a_mute_button() {
        let fake = Fake::bind();
        let mut harness = sink_module(fake.sock.clone());
        let mut conn = fake.accept();
        handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
        let (shown, content) = content_of(&mut harness);
        assert!(shown);
        let widgets = content.widgets();
        assert_eq!(widgets.len(), 3);
        assert_eq!(content.label(&widgets[0]), format!("{DESC}  49%"));
        assert_eq!(
            widgets[1].kind,
            Widget::Slider {
                value: 49,
                max: 100,
                action: "set"
            }
        );
        assert_eq!(content.label(&widgets[2]), "Mute");
        assert_eq!(
            widgets[2].kind,
            Widget::Button {
                action: "toggle-mute",
                arg: None,
                selected: false,
                closes: false,
            }
        );
    }

    #[test]
    fn a_muted_device_offers_unmute() {
        let fake = Fake::bind();
        let mut harness = sink_module(fake.sock.clone());
        let mut conn = fake.accept();
        handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
        assert_eq!(
            harness.invoke(
                &OutputView { name: None },
                &ModuleAction::new("toggle-mute", None),
                1
            ),
            Ok(Update::Unchanged)
        );
        let (tag, _) = conn.expect(proto::CMD_SET_SINK_MUTE);
        conn.ack(tag);
        let _ = wait(&mut harness);
        answer_set(&mut conn, &mut harness, Kind::Sink, SINK, &[VOL, VOL], true);
        let (_, content) = content_of(&mut harness);
        assert_eq!(content.label(&content.widgets()[2]), "Unmute");
    }
}
