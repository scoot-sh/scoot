use super::Theme;
use crate::modules::Class;

#[test]
fn each_class_maps_to_its_token() {
    let theme = Theme::default();
    assert_eq!(theme.class(Class::Normal), theme.foreground);
    assert_eq!(theme.class(Class::Warn), theme.accent);
    assert_eq!(theme.class(Class::Urgent), theme.urgent);
    assert_eq!(theme.class(Class::Muted), theme.dim);
}

#[test]
fn hover_is_the_old_tint_until_set() {
    // The hover tint was the accent before it had a token of its own, so
    // the default keeps it: setting `[colors] hover` is what changes it.
    assert_eq!(Theme::default().hover, Theme::default().accent);
}

#[test]
fn the_classes_are_told_apart_by_default() {
    let theme = Theme::default();
    let mut colors = [
        theme.background,
        theme.class(Class::Normal),
        theme.class(Class::Warn),
        theme.class(Class::Urgent),
        theme.class(Class::Muted),
    ]
    .map(|c| c.xrgb8888());
    colors.sort_unstable();
    colors.windows(2).for_each(|w| assert_ne!(w[0], w[1]));
}
