//! `[wallpaper]`: the section's parse and JSON (`section.rs`), the run
//! queue's rules (`queue.rs`), and the glue on a live `State`: real
//! `apply-config` stand-ins spawned, reaped and logged (`spawn.rs`).

mod queue;
mod section;
mod spawn;
