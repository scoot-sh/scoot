//! The body markup parser: the spec's §4 tags to bounded styled spans.
//!
//! Allowed tags are `b i u a img`; every other tag is stripped with its
//! text kept; entities are decoded (`&amp; &lt; &gt; &quot; &apos;` and
//! numeric `&#N;`/`&#xN;`, an invalid code point becoming U+FFFD, an
//! unknown entity staying literal). An unclosed or malformed tag ends
//! markup parsing, and the rest is plain text. It is never an error, and
//! nothing beyond the five tags is ever interpreted. At most
//! [`MAX_SPANS`] spans: past that the tail joins the last one as plain
//! text, so no content is lost to the bound.

/// The most spans one body parses to: past it the tail is plain text in
/// the last span, never dropped.
pub const MAX_SPANS: usize = 64;

/// One run of text with its style: `a` is underlined accent text, never
/// a link in v1 (a body link that opened a URL on click would be an easy
/// phishing surface); `img` is dropped with its `alt` kept as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// The text, entities decoded, tags stripped.
    pub text: String,
    /// `b`: bold face if configured, else synthetic emboldening.
    pub bold: bool,
    /// `i`: italic face if configured, else upright.
    pub italic: bool,
    /// `u`, and `a` (accent, not clickable).
    pub underline: bool,
}

impl Span {
    /// A plain-text run: what unstyled content and the overflow tail are.
    fn plain() -> Self {
        Self {
            text: String::new(),
            bold: false,
            italic: false,
            underline: false,
        }
    }
}

/// Parses `input` to at most [`MAX_SPANS`] spans. Never fails: hostile
/// content degrades to plain text, never to an error.
pub fn parse(input: &str) -> Vec<Span> {
    let mut parser = Parser {
        chars: input.as_bytes(),
        at: 0,
        spans: vec![Span::plain()],
        bold: 0,
        italic: 0,
        underline: 0,
        capped: false,
    };
    parser.run();
    parser.finish()
}

/// The spans' text concatenated: the plain-text body the JSON view and
/// the history ring keep.
pub fn plain(spans: &[Span]) -> String {
    let mut out = String::new();
    for span in spans {
        out.push_str(&span.text);
    }
    out
}

struct Parser<'a> {
    chars: &'a [u8],
    at: usize,
    spans: Vec<Span>,
    bold: u32,
    italic: u32,
    underline: u32,
    /// Past [`MAX_SPANS`] spans: no new span starts, the tail is text.
    capped: bool,
}

impl Parser<'_> {
    fn run(&mut self) {
        while self.at < self.chars.len() {
            match self.chars[self.at] {
                b'<' => {
                    if !self.tag() {
                        // Malformed or unclosed: the rest (the `<`
                        // included) is plain text.
                        self.rest_plain();
                        return;
                    }
                }
                b'&' => self.entity(),
                _ => {
                    let start = self.at;
                    while self.at < self.chars.len() && !matches!(self.chars[self.at], b'<' | b'&')
                    {
                        self.at += 1;
                    }
                    // The cut is ASCII (`<`/`&`), so always a boundary.
                    self.text(&String::from_utf8_lossy(&self.chars[start..self.at]));
                }
            }
        }
    }

    /// The rest of the input as literal text, entity markers included.
    fn rest_plain(&mut self) {
        if self.at < self.chars.len() {
            let tail = String::from_utf8_lossy(&self.chars[self.at..]).into_owned();
            self.text(&tail);
            self.at = self.chars.len();
        }
    }

    /// The current style's span, starting one past the cap when capped.
    fn current(&mut self) -> &mut Span {
        let bold = self.bold > 0;
        let italic = self.italic > 0;
        let underline = self.underline > 0;
        let matches =
            |span: &Span| span.bold == bold && span.italic == italic && span.underline == underline;
        if !matches(self.spans.last().expect("a span always exists")) {
            if self.spans.len() < MAX_SPANS {
                self.spans.push(Span {
                    text: String::new(),
                    bold,
                    italic,
                    underline,
                });
            } else {
                self.capped = true;
            }
        }
        self.spans.last_mut().expect("a span always exists")
    }

    fn text(&mut self, piece: &str) {
        // Control characters other than tab and newline never reach the
        // screen: dropped here as at the field caps.
        if piece
            .bytes()
            .any(|byte| byte < 0x20 && !matches!(byte, b'\n' | b'\t') || byte == 0x7f)
        {
            let kept: String = piece
                .chars()
                .filter(|char| !matches!(*char, '\0'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}'))
                .collect();
            self.current().text.push_str(&kept);
        } else {
            self.current().text.push_str(piece);
        }
    }

    /// Parses the tag at `at` (which names `<`): `true` when it was a
    /// well-formed tag (known or stripped), `false` when malformed or
    /// unclosed, ending markup parsing with the rest plain.
    fn tag(&mut self) -> bool {
        let mut at = self.at + 1;
        let closing = if self.chars.get(at) == Some(&b'/') {
            at += 1;
            true
        } else {
            false
        };
        let name_start = at;
        while at < self.chars.len() && self.chars[at].is_ascii_alphanumeric() {
            at += 1;
        }
        if at == name_start {
            return false;
        }
        let name = &self.chars[name_start..at];
        // The attributes to the closing `>`, quotes respected so a `>`
        // inside one does not end the tag; an unclosed quote or a missing
        // `>` is malformed.
        let mut alt: Option<String> = None;
        loop {
            while at < self.chars.len()
                && matches!(self.chars[at], b' ' | b'\t' | b'\n' | b'\r' | b'/')
            {
                at += 1;
            }
            if at >= self.chars.len() {
                return false;
            }
            if self.chars[at] == b'>' {
                at += 1;
                break;
            }
            // An attribute: `name[="value"]`; only `img`'s `alt` is read.
            let key_start = at;
            while at < self.chars.len()
                && (self.chars[at].is_ascii_alphanumeric() || self.chars[at] == b'-')
            {
                at += 1;
            }
            if at == key_start {
                return false;
            }
            let key = &self.chars[key_start..at];
            while at < self.chars.len() && self.chars[at] == b' ' {
                at += 1;
            }
            let mut value: &[u8] = &[];
            if self.chars.get(at) == Some(&b'=') {
                at += 1;
                while at < self.chars.len() && self.chars[at] == b' ' {
                    at += 1;
                }
                let quote = *self.chars.get(at).unwrap_or(&0);
                if matches!(quote, b'"' | b'\'') {
                    at += 1;
                    let value_start = at;
                    while at < self.chars.len() && self.chars[at] != quote {
                        at += 1;
                    }
                    if at >= self.chars.len() {
                        return false;
                    }
                    value = &self.chars[value_start..at];
                    at += 1;
                } else {
                    let value_start = at;
                    while at < self.chars.len()
                        && !matches!(self.chars[at], b' ' | b'\t' | b'\n' | b'\r' | b'>')
                    {
                        at += 1;
                    }
                    value = &self.chars[value_start..at];
                }
            }
            if name == b"img" && key.eq_ignore_ascii_case(b"alt") && alt.is_none() {
                alt = Some(decode_entities(&String::from_utf8_lossy(value)));
            }
        }
        self.at = at;
        if name == b"img" {
            if !closing && let Some(alt) = alt {
                self.text(&alt);
            }
            return true;
        }
        let style = match name {
            b"b" => Some(&mut self.bold),
            b"i" => Some(&mut self.italic),
            b"u" | b"a" => Some(&mut self.underline),
            _ => None,
        };
        match (style, closing) {
            // An unknown tag is stripped, its text kept: parsed on.
            (None, _) => true,
            // A stray closer with nothing open is ignored.
            (Some(depth), true) => {
                *depth = depth.saturating_sub(1);
                true
            }
            (Some(depth), false) => {
                *depth = depth.saturating_add(1);
                true
            }
        }
    }

    /// Parses the entity at `at` (which names `&`): a known one decodes,
    /// an unknown one stays literal, and a bare `&` is text.
    fn entity(&mut self) {
        let start = self.at;
        let mut at = start + 1;
        while at < self.chars.len()
            && (self.chars[at].is_ascii_alphanumeric() || self.chars[at] == b'#')
        {
            at += 1;
        }
        if self.chars.get(at) == Some(&b';') && at > start + 1 {
            let body = &self.chars[start + 1..at];
            self.at = at + 1;
            self.text(&decode_entity(body));
        } else {
            self.text("&");
            self.at = start + 1;
        }
    }

    /// Drops trailing empty spans: `<b></b>` is no span at all.
    fn finish(mut self) -> Vec<Span> {
        while self.spans.len() > 1 && self.spans.last().is_some_and(|span| span.text.is_empty()) {
            self.spans.pop();
        }
        self.spans
    }
}

/// Decodes one entity body (between `&` and `;`): the five named ones, a
/// numeric one (an invalid code point becoming U+FFFD), and anything else
/// staying literal with its markers.
fn decode_entity(body: &[u8]) -> String {
    match body {
        b"amp" => String::from("&"),
        b"lt" => String::from("<"),
        b"gt" => String::from(">"),
        b"quot" => String::from("\""),
        b"apos" => String::from("'"),
        _ => {
            if body.first() == Some(&b'#') {
                let number = &body[1..];
                let (digits, base) = match number.first() {
                    Some(b'x') | Some(b'X') => (&number[1..], 16),
                    _ => (number, 10),
                };
                if !digits.is_empty()
                    && let Some(value) = parse_number(digits, base)
                    && let Some(char) = char::from_u32(value)
                {
                    return char.to_string();
                }
                // An invalid code point (out of range, a surrogate, or
                // no digits at all) is U+FFFD, never literal.
                return String::from("\u{FFFD}");
            }
            // An unknown entity stays literal, markers included.
            let mut literal = String::with_capacity(body.len() + 2);
            literal.push('&');
            literal.push_str(&String::from_utf8_lossy(body));
            literal.push(';');
            literal
        }
    }
}

/// Decodes entities in attribute values (the `img` `alt`).
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'&' {
            let mut end = at + 1;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'#') {
                end += 1;
            }
            if bytes.get(end) == Some(&b';') && end > at + 1 {
                out.push_str(&decode_entity(&bytes[at + 1..end]));
                at = end + 1;
                continue;
            }
        }
        // A non-entity byte: the whole char rides. Cutting a
        // multi-byte char would lossy each byte into its own U+FFFD
        // and grow the text past its input (the fuzz target's shrink
        // pin caught exactly that). `at` is always a boundary: it
        // starts at one, entities consume ASCII only, and chars land
        // on the next one — the `get` below says so rather than
        // assuming it.
        let Some(rest) = text.get(at..) else {
            break;
        };
        let Some(char) = rest.chars().next() else {
            break;
        };
        out.push(char);
        at += char.len_utf8();
    }
    out
}

/// Parses ASCII digits in `base`, refusing overflow past `u32`: `None`
/// for an empty run, a bad digit, or a value past `u32::MAX`.
fn parse_number(digits: &[u8], base: u32) -> Option<u32> {
    let mut value: u32 = 0;
    for digit in digits {
        let digit = (*digit as char).to_digit(base)?;
        value = value.checked_mul(base)?.checked_add(digit)?;
    }
    Some(value)
}

/// The fuzz target's check over arbitrary bytes: skipped unless UTF-8,
/// then parsed with the contract pinned — never a panic, at most
/// [`MAX_SPANS`] spans, and no text the input did not carry (entities
/// only shrink, `alt` is a slice of the input).
pub fn fuzz_check(bytes: &[u8]) -> bool {
    let Ok(text) = core::str::from_utf8(bytes) else {
        return false;
    };
    let spans = parse(text);
    assert!(spans.len() <= MAX_SPANS, "past the span cap");
    assert!(!spans.is_empty(), "a parse is never empty");
    let plain_text = plain(&spans);
    assert!(
        plain_text.len() <= text.len(),
        "markup only shrinks text ({})",
        plain_text.len()
    );
    true
}

#[cfg(test)]
mod tests;
