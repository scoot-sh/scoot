//! The microphone variant through the harness: the registry starts it as
//! the bar does, and its sets speak the source commands with the `source`
//! key in `query`. The shared fake server and driving helpers live in
//! `volume::fake`; only what differs from the sink is tested here.

use super::super::volume::fake::{
    Fake, SOURCE, STEP, VOL, answer_set, handshake, mic_module, view_text, wait,
};
use super::super::volume::proto::{self, Bounded, Kind};
use crate::action::ModuleAction;
use crate::modules::{Class, Init, OutputView, Update, find};

#[test]
fn the_registry_starts_the_microphone_variant() {
    let spec = find("microphone").expect("the microphone module is built");
    assert_eq!(spec.id, super::ID);
    assert_eq!(spec.actions, super::ACTIONS);
    let settings = super::super::Settings::default();
    let Init::Available(module) = (spec.init)(&settings) else {
        panic!("microphone starts available");
    };
    let mut harness = crate::modules::harness::Harness::new(module);
    // With no server it waits with nothing shown; with one (a developer
    // machine) the handshake runs itself and a level shows. Either way it
    // answers through the shared keys.
    for _ in 0..10 {
        if !harness.view().is_empty() {
            break;
        }
        let _ = harness.wait(std::time::Duration::from_secs(1));
    }
    let text = view_text(&harness);
    assert!(
        text.is_empty() || text.ends_with('%'),
        "unexpected view {text:?}"
    );
    assert_eq!(harness.value_on(None).is_none(), text.is_empty());
    if let Some(value) = harness.value_on(None) {
        assert!(value.get("source").is_some(), "unexpected value {value}");
    }
}

#[test]
fn raise_sets_the_source_absolutely() {
    let fake = Fake::bind();
    let mut harness = mic_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Source, SOURCE, SOURCE);
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 1),
        Ok(Update::Unchanged)
    );
    // One absolute set from the shown level, on the source command.
    let (tag, payload) = conn.expect(proto::CMD_SET_SOURCE_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    assert_eq!(reader.get_u32(), Some(proto::INVALID_INDEX));
    let mut name = Bounded::empty();
    assert_eq!(
        reader.get_str(&mut name).map(|s| s.map(str::to_owned)),
        Some(Some(SOURCE.to_owned()))
    );
    let mut vols = [0; proto::MAX_CHANNELS];
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[..2], [VOL + STEP, VOL + STEP]);
    // A second raise before the answer coalesces behind it: nothing
    // goes out while a set is in flight.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 1),
        Ok(Update::Unchanged)
    );
    conn.quiet(200);
    // Its answer sends the one queued target, the latest absolute level.
    conn.ack(tag);
    let _ = wait(&mut harness);
    let (tag, payload) = conn.expect(proto::CMD_SET_SOURCE_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    let _ = reader.get_u32();
    let _ = reader.get_str(&mut name);
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[..2], [VOL + 2 * STEP, VOL + 2 * STEP]);
    conn.ack(tag);
    let _ = wait(&mut harness);
    // That answer re-reads: shown is what the server says.
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Source,
        SOURCE,
        &[VOL + 2 * STEP, VOL + 2 * STEP],
        false,
    );
    assert_eq!(view_text(&harness), "59%");
}

#[test]
fn lower_and_toggle_mute_answer_on_the_source() {
    let fake = Fake::bind();
    let mut harness = mic_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Source, SOURCE, SOURCE);
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("lower", None), 2),
        Ok(Update::Unchanged)
    );
    let (tag, payload) = conn.expect(proto::CMD_SET_SOURCE_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    assert_eq!(reader.get_u32(), Some(proto::INVALID_INDEX));
    let mut name = Bounded::empty();
    let _ = reader.get_str(&mut name);
    let mut vols = [0; proto::MAX_CHANNELS];
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[..2], [VOL - 2 * STEP, VOL - 2 * STEP]);
    conn.ack(tag);
    let _ = wait(&mut harness);
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Source,
        SOURCE,
        &[VOL - 2 * STEP, VOL - 2 * STEP],
        false,
    );
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("toggle-mute", None), 1),
        Ok(Update::Unchanged)
    );
    let (tag, _) = conn.expect(proto::CMD_SET_SOURCE_MUTE);
    conn.ack(tag);
    let _ = wait(&mut harness);
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Source,
        SOURCE,
        &[VOL - 2 * STEP, VOL - 2 * STEP],
        true,
    );
    assert_eq!(harness.view().class(), Class::Muted);
    assert_eq!(
        harness.value_on(None),
        Some(serde_json::json!({"volume": 39, "muted": true, "source": SOURCE}))
    );
}
