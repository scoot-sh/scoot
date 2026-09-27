use super::{Filter, Mode};

#[test]
fn modes_and_filters_round_trip_by_name() {
    for mode in Mode::ALL {
        assert_eq!(Mode::from_name(mode.name()), Some(mode));
    }
    for filter in Filter::ALL {
        assert_eq!(Filter::from_name(filter.name()), Some(filter));
    }
    assert_eq!(Mode::default(), Mode::Fill);
    assert_eq!(Filter::default(), Filter::Lanczos3);
}

#[test]
fn names_are_exact() {
    for bad in ["", "Fill", "FILL", " fill", "fill ", "centre", "lanczos"] {
        assert_eq!(Mode::from_name(bad), None, "{bad:?}");
        assert_eq!(Filter::from_name(bad), None, "{bad:?}");
    }
}

#[test]
fn they_serialize_as_their_names() {
    assert_eq!(serde_json::to_string(&Mode::Center).unwrap(), "\"center\"");
    assert_eq!(
        serde_json::to_string(&Filter::CatmullRom).unwrap(),
        "\"catmull-rom\""
    );
}
