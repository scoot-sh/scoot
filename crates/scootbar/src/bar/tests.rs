use super::{
    Anchors, Bar, DEFAULT_HEIGHT, Edge, MAX_HEIGHT, MAX_MARGIN, Margin, MarginError, parse_height,
};

fn margin(top: u32, right: u32, bottom: u32, left: u32) -> Margin {
    Margin {
        top,
        right,
        bottom,
        left,
    }
}

#[test]
fn margins_follow_css_shorthand() {
    assert_eq!(Margin::parse("8"), Ok(margin(8, 8, 8, 8)));
    assert_eq!(Margin::parse("8,4"), Ok(margin(8, 4, 8, 4)));
    assert_eq!(Margin::parse("8,4,2"), Ok(margin(8, 4, 2, 4)));
    assert_eq!(Margin::parse("8,4,2,1"), Ok(margin(8, 4, 2, 1)));
    assert_eq!(Margin::parse("0"), Ok(Margin::default()));
}

#[test]
fn a_malformed_margin_is_refused() {
    for text in [
        "",
        ",",
        "8,",
        ",8",
        "8,,4",
        "8,4,2,1,0",
        "-1",
        "+1",
        " 8",
        "8 ",
        "8 4",
        "x",
        "1.5",
        "8;4",
    ] {
        assert_eq!(Margin::parse(text), Err(MarginError::Malformed), "{text:?}");
    }
}

#[test]
fn a_margin_is_bounded() {
    assert_eq!(
        Margin::parse(&MAX_MARGIN.to_string()).map(|m| m.top),
        Ok(MAX_MARGIN)
    );
    assert_eq!(
        Margin::parse(&(MAX_MARGIN + 1).to_string()),
        Err(MarginError::TooLarge)
    );
    // Past `u32` is not a number at all.
    assert_eq!(
        Margin::parse("99999999999999999999"),
        Err(MarginError::Malformed)
    );
    assert_eq!(Margin::parse("0,2000"), Err(MarginError::TooLarge));
}

#[test]
fn a_height_is_1_to_the_maximum() {
    assert_eq!(parse_height("1"), Some(1));
    assert_eq!(parse_height("28"), Some(28));
    assert_eq!(parse_height(&MAX_HEIGHT.to_string()), Some(MAX_HEIGHT));
    for text in [
        "0",
        "",
        "-28",
        "+28",
        "28px",
        " 28",
        "1025",
        "99999999999999999999",
    ] {
        assert_eq!(parse_height(text), None, "{text:?}");
    }
}

#[test]
fn edges_parse_by_name() {
    assert_eq!(Edge::parse("top"), Some(Edge::Top));
    assert_eq!(Edge::parse("bottom"), Some(Edge::Bottom));
    for text in ["", "Top", "left", "right", "top "] {
        assert_eq!(Edge::parse(text), None, "{text:?}");
    }
}

#[test]
fn the_default_bar_is_a_top_bar() {
    let bar = Bar::default();
    assert_eq!(bar.edge, Edge::Top);
    assert_eq!(bar.height, DEFAULT_HEIGHT);
    assert_eq!(bar.margin, Margin::default());
}

#[test]
fn a_bar_is_anchored_to_its_edge_and_both_sides() {
    let top = Bar::default();
    assert_eq!(
        top.anchors(),
        Anchors {
            top: true,
            bottom: false,
            left: true,
            right: true
        }
    );
    let bottom = Bar {
        edge: Edge::Bottom,
        ..Bar::default()
    };
    assert_eq!(
        bottom.anchors(),
        Anchors {
            top: false,
            bottom: true,
            left: true,
            right: true
        }
    );
}

#[test]
fn the_zone_is_the_height_whatever_the_margin() {
    let bar = Bar {
        height: 30,
        margin: margin(8, 8, 8, 8),
        ..Bar::default()
    };
    // The compositor adds the anchored edge's margin itself.
    assert_eq!(bar.exclusive_zone(), 30);
    assert_eq!(bar.requested_size(), (0, 30));
    assert_eq!(bar.margins(), [8, 8, 8, 8]);
    let tallest = Bar {
        height: MAX_HEIGHT,
        ..Bar::default()
    };
    assert_eq!(tallest.exclusive_zone(), MAX_HEIGHT as i32);
}

#[test]
fn margins_are_sent_top_right_bottom_left() {
    let bar = Bar {
        margin: margin(1, 2, 3, 4),
        ..Bar::default()
    };
    assert_eq!(bar.margins(), [1, 2, 3, 4]);
}

#[test]
fn a_fallback_width_takes_off_the_side_margins_and_is_never_zero() {
    let bar = Bar {
        margin: margin(0, 10, 0, 20),
        ..Bar::default()
    };
    assert_eq!(bar.width_on(1920), 1890);
    assert_eq!(bar.width_on(30), 1);
    assert_eq!(bar.width_on(0), 1);
    let wide = Bar {
        margin: margin(0, MAX_MARGIN, 0, MAX_MARGIN),
        ..Bar::default()
    };
    assert_eq!(wide.width_on(u32::MAX), u32::MAX - 2 * MAX_MARGIN);
}
