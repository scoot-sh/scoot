//! The spacing options: `bar.separator`, a module's `margin`, and the
//! workspaces pill's shape. Each is bounded, and a hostile value is a loud
//! refusal naming its key rather than a size the layout must survive.

use super::tests::read;
#[cfg(feature = "workspaces")]
use crate::modules::workspaces::Shape;

#[test]
fn the_defaults_add_nothing() {
    let config = read("").unwrap();
    assert_eq!(config.layout.separator, 0);
    assert!(config.layout.margins.is_empty());
    assert_eq!(config.style().separator, 0);
    #[cfg(feature = "workspaces")]
    assert_eq!(config.modules.workspaces.pill, Default::default());
}

#[test]
fn a_separator_is_a_line_that_fits_in_the_spacing() {
    let config = read("[bar]\nspacing = 6\nseparator = 2\n").unwrap();
    assert_eq!(config.layout.separator, 2);
    assert_eq!(config.style().separator, 2);
    // Exactly the spacing is the widest.
    assert!(read("[bar]\nspacing = 2\nseparator = 2\n").is_ok());
    for text in [
        "[bar]\nseparator = 1\n",
        "[bar]\nspacing = 2\nseparator = 3\n",
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(
            error.contains("'bar.separator'") && error.contains("bar.spacing"),
            "{text:?}: {error}"
        );
    }
}

#[test]
fn hostile_spacing_values_are_refused_naming_the_key() {
    for text in [
        "[bar]\nseparator = -1\n",
        "[bar]\nseparator = 1.5\n",
        "[bar]\nseparator = \"2\"\n",
        "[bar]\nspacing = 1025\nseparator = 1025\n",
        "[bar]\nspacing = 1024\nseparator = 1025\n",
        "[bar]\nseparator = 4294967296\n",
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(
            error.contains("separator") || error.contains("spacing"),
            "{text:?}: {error}"
        );
    }
    assert!(read("[bar]\nspacing = 1024\nseparator = 1024\n").is_ok());
}

#[test]
#[cfg(feature = "clock")]
fn a_module_margin_is_bounded_and_listed_only_when_set() {
    let config = read("[clock]\nmargin = 6\n").unwrap();
    assert_eq!(config.layout.margins, [("clock", 6)]);
    assert_eq!(config.layout.margin_of("clock"), 6);
    assert!(
        read("[clock]\nmargin = 0\n")
            .unwrap()
            .layout
            .margins
            .is_empty()
    );
    assert!(read("[clock]\nmargin = 1024\n").is_ok());
    for text in [
        "[clock]\nmargin = 1025\n",
        "[clock]\nmargin = -1\n",
        "[clock]\nmargin = 1.5\n",
        "[clock]\nmargin = \"4\"\n",
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains("margin"), "{text:?}: {error}");
    }
}

#[test]
#[cfg(feature = "workspaces")]
fn a_workspaces_margin_and_pill_are_bounded() {
    let config = read("[workspaces]\nmargin = 3\npill-radius = 8\npill-inset = 2\n").unwrap();
    assert_eq!(config.layout.margins, [("workspaces", 3)]);
    let pill = config.modules.workspaces.pill;
    assert_eq!((pill.shape, pill.radius, pill.inset), (Shape::Rect, 8, 2));
    for text in [
        "[workspaces]\nmargin = 1025\n",
        "[workspaces]\npill-radius = 1025\n",
        "[workspaces]\npill-inset = 1025\n",
        "[workspaces]\npill-radius = -1\n",
        "[workspaces]\npill-inset = 0.5\n",
        "[workspaces]\npill-radius = \"8\"\n",
        "[workspaces]\npill-shape = \"oval\"\n",
        "[workspaces]\npill-shape = \"Circle\"\n",
        "[workspaces]\npill-shape = 1\n",
        "[workspaces]\npill-shape = \"pill\"\npill-radius = 4\n",
        "[workspaces]\npill-shape = \"circle\"\npill-radius = 0\n",
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(
            error.contains("margin") || error.contains("pill"),
            "{text:?}: {error}"
        );
    }
}

#[test]
#[cfg(all(feature = "clock", feature = "workspaces"))]
fn margins_survive_a_layout_that_names_the_modules() {
    let config = read(
        "left = [\"workspaces\"]\nright = [\"clock\"]\n[clock]\nmargin = 2\n[workspaces]\nmargin = 5\n",
    )
    .unwrap();
    assert_eq!(config.layout.margin_of("clock"), 2);
    assert_eq!(config.layout.margin_of("workspaces"), 5);
}

#[test]
#[cfg(feature = "workspaces")]
fn the_pill_shapes_are_read_by_name() {
    for (name, shape) in [
        ("rect", Shape::Rect),
        ("pill", Shape::Pill),
        ("circle", Shape::Circle),
    ] {
        let config = read(&format!("[workspaces]\npill-shape = \"{name}\"\n")).unwrap();
        assert_eq!(config.modules.workspaces.pill.shape, shape);
    }
    // A radius goes with a rect, spelled out or by default.
    assert!(read("[workspaces]\npill-shape = \"rect\"\npill-radius = 4\n").is_ok());
    // The inset goes with any.
    assert!(read("[workspaces]\npill-shape = \"circle\"\npill-inset = 3\n").is_ok());
    let error = read("[workspaces]\npill-shape = \"pill\"\npill-radius = 4\n")
        .unwrap_err()
        .to_string();
    assert!(error.contains("'workspaces.pill-radius'"), "{error}");
}

#[test]
#[cfg(feature = "workspaces")]
fn the_workspaces_item_gap_is_a_count_of_spaces() {
    assert_eq!(
        read("").unwrap().modules.workspaces.item_gap,
        1,
        "the default is one space"
    );
    for spaces in [1, 4, 8] {
        let config = read(&format!("[workspaces]\nitem-gap = {spaces}\n")).unwrap();
        assert_eq!(config.modules.workspaces.item_gap, spaces);
    }
    for text in [
        "[workspaces]\nitem-gap = 0\n",
        "[workspaces]\nitem-gap = 9\n",
        "[workspaces]\nitem-gap = -1\n",
        "[workspaces]\nitem-gap = 1.5\n",
        "[workspaces]\nitem-gap = \"2\"\n",
    ] {
        let error = read(text).unwrap_err().to_string();
        assert!(error.contains("item-gap"), "{text:?}: {error}");
    }
}
