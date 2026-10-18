//! A tooltip's text as lines: broken at a width, bounded in count, so one
//! long window title is a block a few lines tall rather than a strip wider
//! than the screen.

use super::{Content, Kind, MAX_TEXT};

/// The most lines a tooltip shows. What is left is cut, with an ellipsis on
/// the last line.
pub const MAX_LINES: usize = 6;

impl Content {
    /// Fills this (cleared first) with `tip` broken into text lines no wider
    /// than `max` as `measure` counts, at most [`MAX_LINES`]. Breaks at
    /// spaces and at a newline in `tip`; a word wider than `max` is broken
    /// where it fills the line. A control character is dropped. Nothing is
    /// allocated once the content's storage has grown to its bounds.
    pub fn wrap_tooltip(&mut self, tip: &str, max: u32, measure: impl Fn(&str) -> u32) {
        self.clear();
        let mut cut = false;
        'paragraphs: for paragraph in tip.split('\n') {
            let mut rest = paragraph.trim_matches(|c: char| c.is_whitespace() || c.is_control());
            while !rest.is_empty() {
                if self.widgets.len() >= MAX_LINES || self.text.len() >= MAX_TEXT {
                    cut = true;
                    break 'paragraphs;
                }
                let end = line_end(rest, max, &measure);
                let (line, after) = rest.split_at(end);
                self.push_line(line);
                rest = after.trim_start_matches(|c: char| c.is_whitespace() || c.is_control());
            }
        }
        if cut {
            self.ellipsize_last(max, &measure);
        }
    }

    fn push_line(&mut self, line: &str) {
        let start = self.text.len();
        for c in line.chars().filter(|c| !c.is_control()) {
            if self.text.len() + c.len_utf8() > MAX_TEXT {
                break;
            }
            self.text.push(c);
        }
        let end = self.text.len();
        self.widgets.push(super::Widget {
            kind: Kind::Text,
            label: start..end,
        });
    }

    /// Ends the last line in `…`, dropping characters until it fits.
    fn ellipsize_last(&mut self, max: u32, measure: &impl Fn(&str) -> u32) {
        let Some(last) = self.widgets.last().map(|w| w.label.clone()) else {
            return;
        };
        let mut end = last.end;
        loop {
            self.text.truncate(end);
            self.text.push('…');
            let fits = measure(self.text.get(last.start..).unwrap_or("")) <= max;
            if fits || end <= last.start {
                break;
            }
            // Drop one character, on a boundary.
            end -= 1;
            while !self.text.is_char_boundary(end) {
                end -= 1;
            }
        }
        let total = self.text.len();
        if let Some(widget) = self.widgets.last_mut() {
            widget.label = last.start..total;
        }
    }
}

/// How many bytes of `line` (no newline, no leading space) make the next
/// row: all of it if it fits, else up to the last space that fits, else as
/// many characters as fit (at least one, so progress is certain).
fn line_end(line: &str, max: u32, measure: &impl Fn(&str) -> u32) -> usize {
    if measure(line) <= max {
        return line.len();
    }
    let mut last_space = None;
    let mut last_fit = 0;
    for (at, c) in line.char_indices() {
        let end = at + c.len_utf8();
        if measure(line.get(..end).unwrap_or(line)) > max {
            break;
        }
        last_fit = end;
        if c == ' ' {
            last_space = Some(at);
        }
    }
    match last_space {
        // A break at a space that is not the very start of the line.
        Some(at) if at > 0 => at,
        _ if last_fit > 0 => last_fit,
        _ => line.chars().next().map_or(line.len(), char::len_utf8),
    }
}
