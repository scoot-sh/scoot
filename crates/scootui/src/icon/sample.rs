//! Resampling premultiplied pixels into a square: the filter an image
//! icon and a tray pixmap share, so neither duplicates it.
//!
//! The source is premultiplied `b, g, r, a`, rows packed. To fit a
//! square of `side` pixels, keeping the aspect ratio and centered, with
//! transparent margins. The filter is a separable **triangle** (bilinear)
//! filter over *premultiplied* color, its support widened to the scale
//! ratio when shrinking so every source pixel counts (an area average,
//! not point sampling), and plain bilinear when enlarging.
//! Premultiplied, so a transparent pixel's color never bleeds into a
//! neighbor as a dark or light fringe.

/// Fits `pixels` (`width × height` premultiplied `b, g, r, a`, rows
/// packed) into `out`, `side × side` premultiplied pixels. `out` shorter
/// than that is left untouched; a zero source or side draws nothing.
pub fn scale_into(pixels: &[u8], width: usize, height: usize, side: u32, out: &mut [u8]) {
    let side = side as usize;
    let Some(out) = out.get_mut(..side * side * 4) else {
        return;
    };
    out.fill(0);
    if side == 0 || width == 0 || height == 0 {
        return;
    }
    // The size drawn: the image's aspect ratio in a square.
    let fit = |long: usize, other: usize| ((other * side + long / 2) / long).clamp(1, side);
    let (dw, dh) = if width >= height {
        (side, fit(width, height))
    } else {
        (fit(height, width), side)
    };
    let horizontal = Taps::new(width, dw);
    let vertical = Taps::new(height, dh);
    // Across, into `dw × h`, then down into `dw × dh`.
    let mut across = vec![0.0f32; dw * height * 4];
    for y in 0..height {
        for x in 0..dw {
            let (start, weights) = horizontal.at(x);
            let mut sum = [0.0f32; 4];
            for (i, &weight) in weights.iter().enumerate() {
                let at = (y * width + start + i) * 4;
                if let Some(p) = pixels.get(at..at + 4) {
                    for (s, &c) in sum.iter_mut().zip(p) {
                        *s += weight * f32::from(c);
                    }
                }
            }
            let to = (y * dw + x) * 4;
            if let Some(slot) = across.get_mut(to..to + 4) {
                slot.copy_from_slice(&sum);
            }
        }
    }
    let (left, top) = ((side - dw) / 2, (side - dh) / 2);
    for y in 0..dh {
        let (start, weights) = vertical.at(y);
        for x in 0..dw {
            let mut sum = [0.0f32; 4];
            for (i, &weight) in weights.iter().enumerate() {
                let at = ((start + i) * dw + x) * 4;
                if let Some(p) = across.get(at..at + 4) {
                    for (s, &c) in sum.iter_mut().zip(p) {
                        *s += weight * c;
                    }
                }
            }
            let to = ((top + y) * side + left + x) * 4;
            if let Some(slot) = out.get_mut(to..to + 4) {
                for (d, s) in slot.iter_mut().zip(sum) {
                    *d = (s + 0.5).clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// The filter's taps for resampling `from` samples to `to`: for each
/// output sample, the first source sample and its normalized weights.
struct Taps {
    /// `(first source index, offset into weights, count)` per output.
    spans: Vec<(usize, usize, usize)>,
    weights: Vec<f32>,
}

impl Taps {
    fn new(from: usize, to: usize) -> Self {
        let from = from.max(1);
        let ratio = from as f32 / to.max(1) as f32;
        // Wider than a sample when shrinking: every source sample counts.
        let radius = ratio.max(1.0);
        let mut spans = Vec::with_capacity(to);
        let mut weights = Vec::new();
        for x in 0..to {
            let center = (x as f32 + 0.5) * ratio - 0.5;
            let lo = (((center - radius).floor() + 1.0).max(0.0) as usize).min(from - 1);
            let hi = (((center + radius).ceil() - 1.0).max(0.0) as usize).min(from - 1);
            let offset = weights.len();
            let mut total = 0.0f32;
            for i in lo..=hi.max(lo) {
                let w = (1.0 - (i as f32 - center).abs() / radius).max(0.0);
                weights.push(w);
                total += w;
            }
            if total > 0.0 {
                for w in &mut weights[offset..] {
                    *w /= total;
                }
            } else {
                // No tap reached (cannot happen for a real ratio): the
                // first sample alone.
                weights.truncate(offset);
                weights.push(1.0);
            }
            spans.push((lo, offset, weights.len() - offset));
        }
        Self { spans, weights }
    }

    fn at(&self, x: usize) -> (usize, &[f32]) {
        match self.spans.get(x) {
            Some(&(start, offset, count)) => (
                start,
                self.weights.get(offset..offset + count).unwrap_or(&[]),
            ),
            None => (0, &[]),
        }
    }
}
