//! The integer scale of each bound output, and which of them a surface is
//! on: the fallback under a surface's own scale events, for a compositor
//! that sends neither `preferred_buffer_scale` nor a fractional scale.
//!
//! A surface opened on a null output is shown wherever the compositor
//! chooses, so which output backs it up is the one `wl_surface.enter`
//! names. Generic over the output handle (a `wl_output` in the apps), so
//! the arithmetic is tested without a connection.

/// The integer scale of each bound output, and which of them the surface is
/// on. Generic over the output handle so the arithmetic is tested without a
/// connection.
#[derive(Debug)]
pub struct OutputScales<K> {
    known: Vec<(K, i32)>,
    entered: Vec<K>,
}

impl<K> Default for OutputScales<K> {
    fn default() -> Self {
        Self {
            known: Vec::new(),
            entered: Vec::new(),
        }
    }
}

impl<K: PartialEq + Clone> OutputScales<K> {
    /// An output was bound: scale 1 until it says otherwise.
    pub fn bound(&mut self, output: K) {
        if !self.known.iter().any(|(known, _)| *known == output) {
            self.known.push((output, 1));
        }
    }

    /// `wl_output.scale`. A factor below 1 is a broken compositor's, and
    /// ignored (the scale divides).
    pub fn set_scale(&mut self, output: &K, factor: i32) {
        if factor < 1 {
            return;
        }
        if let Some((_, scale)) = self.known.iter_mut().find(|(known, _)| known == output) {
            *scale = factor;
        }
    }

    /// The output went away (`wl_registry.global_remove`): it is neither
    /// known nor entered any more.
    pub fn remove(&mut self, output: &K) {
        self.known.retain(|(known, _)| known != output);
        self.leave(output);
    }

    /// `wl_surface.enter`.
    pub fn enter(&mut self, output: &K) {
        if !self.entered.contains(output) {
            self.entered.push(output.clone());
        }
    }

    /// `wl_surface.leave`.
    pub fn leave(&mut self, output: &K) {
        self.entered.retain(|entered| entered != output);
    }

    /// The surface is gone: it is on no output.
    pub fn leave_all(&mut self) {
        self.entered.clear();
    }

    /// The scale to fall back on: the largest of the outputs the surface is
    /// on (drawn larger and scaled down is sharp; the other way is soft),
    /// `None` while it is on none, so nothing is guessed.
    pub fn current(&self) -> Option<i32> {
        self.entered
            .iter()
            .map(|entered| {
                self.known
                    .iter()
                    .find(|(known, _)| known == entered)
                    .map_or(1, |(_, scale)| *scale)
            })
            .max()
    }
}

#[cfg(test)]
mod tests;
