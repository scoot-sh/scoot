//! Cutting a line of text to a span with an ellipsis, measured in device
//! pixels and never counted in characters: the window title's cut, shared
//! with the media module's (the two lines a bar shows that are arbitrary
//! text from outside).

use super::MAX_TEXT;
use crate::text::Text;

/// What fitting `full` into a span came to.
pub enum Cut<'b> {
    /// It fits whole: the plain draw is right.
    Fits,
    /// Not even the ellipsis fits: the span is left blank.
    Blank,
    /// The longest prefix that leaves room for the ellipsis, with it.
    Shown(&'b str),
}

/// `full` cut to `available` device pixels at `em`, into `buf` (so a cut
/// allocates nothing). Advances are fractional and each glyph starts
/// rounded, so the draw's clip is the backstop for the last pixel.
pub fn cut<'b>(
    text: &Text,
    full: &str,
    em: f32,
    available: u32,
    buf: &'b mut [u8; MAX_TEXT],
) -> Cut<'b> {
    if text.measure(None, full, em) <= available {
        return Cut::Fits;
    }
    const ELLIPSIS: char = '…';
    let ellipsis = text.measure(None, "…", em);
    if ellipsis > available {
        return Cut::Blank;
    }
    let budget = available - ellipsis;
    let mut len = 0usize;
    let mut pen = 0.0f32;
    for c in full.chars().filter(|c| !c.is_control()) {
        let next = pen + text.advance(c, em);
        if next.round() > budget as f32 {
            break;
        }
        let width = c.len_utf8();
        if len + width + ELLIPSIS.len_utf8() > buf.len() {
            break;
        }
        c.encode_utf8(&mut buf[len..]);
        len += width;
        pen = next;
    }
    ELLIPSIS.encode_utf8(&mut buf[len..]);
    len += ELLIPSIS.len_utf8();
    Cut::Shown(std::str::from_utf8(&buf[..len]).unwrap_or(""))
}
