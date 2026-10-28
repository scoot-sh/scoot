---
title: "Audit the popup and icon zero-allocation pins that ran without a counting allocator"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-27"
---

# Audit the popup and icon zero-allocation pins that ran without a counting allocator

Filed 2026-10-18. Serves **daily-drive (the bar's zero-allocation bar is a stated standard)**.

## The gap

PRs #670 and #671 found that `scootui`'s lib test binary had no counting `#[global_allocator]`, so `scootbg_mem::count_allocations` always read 0 there. #670 installed one (`crates/scootui/src/lib.rs`, `cfg(test)`). The older pins that ran under it before then (popup tip/tests, icon cache and raster) were vacuous until now; they pass today but have never been mutation-proved.

## What to do

For each `count_allocations` pin in scootui (popup, icon), inject a `format!` into the measured path, see it fail, revert, and record it. Fix any pin that turns out to measure nothing.

## Not in this ticket

scootbar's own pins: its binary crate installs the allocator and its pins were already real.

## Resolution (2026-10-27)

Every `count_allocations` pin in `crates/scootui` was mutation-proved:
`std::hint::black_box(Box::new(7u8));` inserted as the first statement of
the production function the pin's closure calls (not in the test), the
pin run alone, the file restored from a backup and `cmp`-checked
byte-identical. Run on the dev VM against a copy of origin/main
`203f95300`, each pin with the feature set CI uses for it (`entry`,
`keyboard`, `surfaces` are feature-gated: a bare `cargo nextest run -p
scootui` does not build those pins; CI runs each feature alone).

Result: **every pin was already real, none measured nothing.** The
lib-test allocator (`lib.rs`, `cfg(test)`) is the only one in the crate;
the two integration targets (`tests/keyboard.rs`, `tests/surfaces.rs`)
make no `count_allocations` call, so no pin runs without an allocator
today.

| Pin (test) | Mutated production fn | Result |
| --- | --- | --- |
| `icon::cache::tests::a_second_ask_at_the_same_size_is_a_hit_and_allocates_nothing` | `Cache::get` | red (1) |
| `icon::raster::tests::the_scratch_is_reused_between_fills` | `Rasterizer::fill` | red (1) |
| `popup::tests::a_warm_popup_allocates_nothing` | `Layout::compute`, `paint`, `State::{motion,press,release,retain,scroll_by,move_selection,keyboard_enter}` (9 mutations) | all red (200 or 400) |
| `popup::tip_tests::a_warm_hover_and_a_shown_tooltip_allocate_nothing` | `Hover::{update,wait,dismiss}`, `wrap_tooltip`, `Layout::compute_tooltip`, `paint` (6) | all red (60 or 300) |
| `surfaces::tests::a_hot_turn_allocates_nothing` | `Demand::{show,hide,configure,plan,drew,committed}` (6) | all red (300) |
| `entry::tests::clear_keeps_the_buffer_for_the_next_fill` | `Entry::insert_text` | red (1) |
| `entry::tests::keystrokes_allocate_nothing_after_warm_up` | `Entry::{insert_text,paste,copy_text,move_word_left,backspace,delete_word_back,clear}` (7) | all red (4 or 8) |
| `entry::tests::password_keystrokes_allocate_nothing` | `PasswordEntry::{insert_text,move_word_left,backspace,clear}` (4) | all red (3) |
| `entry::tests::scroll_ops_allocate_nothing_after_warm_up` | `List::{select,select_next,page_down,scroll_to,set_highlights,highlights_for}` (6) | all red (500) |
| `keyboard::tests::hot_path_allocates_nothing` (plain closure) | `update_modifiers`, `keysym`, `text_into`, `direct_utf8`, `caps_lock` (5) | all red ("modifiers, keysym, text and queries allocate") |
| `keyboard::tests::hot_path_allocates_nothing` (composed closure) | `text_into`'s Composed arm, `raw::compose_utf8_into` (2) | red ("composed text allocates nothing per event") |
| `keyboard::tests::seat_event_path_allocates_nothing` | `KeyboardSeat::{key_event,modifiers_event,caps_lock}` (3) | all red (1 or 2) |

Raw output: the `FAIL`/`left:`/`right:` lines for each of the 51
mutations, captured on the VM in `/dev/shm/d2/mut.log`, `mut2a.log` and
`mut3.log`; the three reruns that distinguish the two keyboard closures
are quoted above.

### The class is now closed

The audit found no vacuous pin, but also nothing stopping the next one:
a `count_allocations` call in a test binary without the allocator (an
integration target, a new crate) reads 0 and passes. `count_allocations`
now makes one probe allocation after arming and panics if it went
uncounted ("CountingAlloc is not the global allocator of this test
binary"). Failing first: `scootbg-mem/tests/uninstalled.rs` (a binary
with no allocator) failed with "test did not panic as expected" before the
change and passes after. Every other user of `count_allocations`
(`scootbar`, `scootlaunch`, `scootnotify`, `scoot-dbus`, `scootui`) ran
green under the probe, so none of their pins was vacuous either.
