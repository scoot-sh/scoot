//! The markup contract: the five tags, stripped tags, entities, the
//! span cap, and malformed input degrading to text.

use super::{MAX_SPANS, fuzz_check, parse, plain};

#[test]
fn plain_text_is_one_span() {
    let spans = parse("hello");
    assert_eq!(spans.len(), 1);
    assert_eq!(plain(&spans), "hello");
    assert!(!spans[0].bold && !spans[0].italic && !spans[0].underline);
}

#[test]
fn the_five_tags_style() {
    let spans = parse("<b>bold</b> <i>it</i> <u>un</u> <a href=\"http://x\">link</a>");
    let text = plain(&spans);
    assert_eq!(text, "bold it un link");
    assert!(
        spans
            .iter()
            .find(|span| span.text == "bold")
            .is_some_and(|span| span.bold)
    );
    assert!(
        spans
            .iter()
            .find(|span| span.text == "it")
            .is_some_and(|span| span.italic)
    );
    assert!(
        spans
            .iter()
            .find(|span| span.text == "un")
            .is_some_and(|span| span.underline)
    );
    // A link is underlined text, never the URL.
    let link = spans.iter().find(|span| span.text == "link").unwrap();
    assert!(link.underline && !link.bold && !link.italic);
    assert!(!text.contains("http"));
}

#[test]
fn unknown_tags_strip_with_their_text_kept() {
    assert_eq!(plain(&parse("a<foo> b </foo>c")), "a b c");
    assert_eq!(plain(&parse("a<br/>c")), "ac");
}

#[test]
fn img_drops_with_its_alt_kept() {
    assert_eq!(plain(&parse("a<img src=\"x\" alt=\"pic\">c")), "apicc");
    assert_eq!(plain(&parse("a<img src=\"x\">c")), "ac");
    assert_eq!(plain(&parse("a<img alt=\"A &amp; B\">c")), "aA & Bc");
}

#[test]
fn entities_decode() {
    assert_eq!(
        plain(&parse("a &amp; b &lt;c&gt; &quot;q&quot; &apos;s&apos;")),
        "a & b <c> \"q\" 's'"
    );
    assert_eq!(plain(&parse("&#65;&#x42;")), "AB");
}

#[test]
fn an_invalid_code_point_is_ufffd() {
    assert_eq!(plain(&parse("&#xD800;")), "\u{FFFD}");
    assert_eq!(plain(&parse("&#;")), "\u{FFFD}");
    assert_eq!(plain(&parse("&#99999999;")), "\u{FFFD}");
}

#[test]
fn an_unknown_entity_stays_literal() {
    assert_eq!(plain(&parse("a &bogus; b")), "a &bogus; b");
    assert_eq!(plain(&parse("a & b")), "a & b");
}

#[test]
fn an_unclosed_tag_ends_markup_with_the_rest_plain() {
    assert_eq!(plain(&parse("ok <b>bold <i>tail")), "ok bold tail");
    let spans = parse("ok <b>bold <i>tail");
    assert!(
        spans
            .iter()
            .any(|span| span.text.contains("bold") && span.bold)
    );
}

#[test]
fn a_malformed_tag_ends_markup_with_the_rest_plain() {
    assert_eq!(plain(&parse("a <3 b")), "a <3 b");
    assert_eq!(plain(&parse("a < b")), "a < b");
}

#[test]
fn nesting_stacks() {
    let spans = parse("<b>a <i>b</i> c</b>");
    assert_eq!(plain(&spans), "a b c");
    let both = spans.iter().find(|span| span.text == "b").unwrap();
    assert!(both.bold && both.italic);
    let outer = spans.iter().find(|span| span.text.contains("c")).unwrap();
    assert!(outer.bold && !outer.italic);
}

#[test]
fn a_stray_closer_is_ignored() {
    assert_eq!(plain(&parse("a</b>c")), "ac");
}

#[test]
fn empty_tags_leave_no_span() {
    assert_eq!(parse("<b></b>"), parse(""));
    assert_eq!(
        parse(""),
        vec![super::Span {
            text: String::new(),
            bold: false,
            italic: false,
            underline: false,
        }]
    );
}

#[test]
fn control_characters_drop() {
    assert_eq!(plain(&parse("a\x07b\tc\nd")), "ab\tc\nd");
}

#[test]
fn past_64_spans_the_tail_is_text_not_lost() {
    let mut input = String::new();
    for index in 0..100 {
        input.push_str(&format!("<b>{index}</b>"));
    }
    let spans = parse(&input);
    assert!(spans.len() <= MAX_SPANS, "{}", spans.len());
    let text: String = (0..100).map(|index| index.to_string()).collect();
    assert_eq!(plain(&spans), text);
}

/// Reverting the cap check to always push leaves this failing at 101
/// spans: the bound is load-bearing.
#[test]
fn the_check_spots_a_seed() {
    assert!(fuzz_check(b"<b>hi</b>"));
    assert!(!fuzz_check(b"\xff\xfe"));
}

#[test]
fn the_committed_corpus_replays_on_stable() {
    crate::replay_corpus("markup", true, fuzz_check);
}
