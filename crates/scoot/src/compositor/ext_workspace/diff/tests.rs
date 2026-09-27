use super::Change::{Added, Removed, Renamed, Restated};
use super::*;

fn snapshot(count: usize, active: usize) -> Workspaces {
    Workspaces { count, active }
}

/// The changes between two snapshots, as a `Vec` -- the production caller
/// reuses one buffer, which these don't need to.
fn between(published: Workspaces, current: Workspaces) -> Vec<Change> {
    let mut out = Vec::new();
    changes(published, current, &mut out);
    out
}

#[test]
fn nothing_changed_produces_nothing() {
    // The condition the caller also uses to decide not to send `done`: an
    // empty list and "the snapshots are equal" have to mean the same thing.
    for (count, active) in [(1, 0), (2, 0), (2, 1), (7, 6)] {
        let same = snapshot(count, active);
        assert_eq!(between(same, same), &[], "{count}/{active}");
    }
}

#[test]
fn the_first_snapshot_creates_every_workspace() {
    // What a compositor that has just created its output publishes, against
    // the default `Workspaces` nothing has been told about yet.
    assert_eq!(
        between(Workspaces::default(), snapshot(3, 1)),
        &[
            Added {
                index: 0,
                active: false
            },
            Added {
                index: 1,
                active: true
            },
            Added {
                index: 2,
                active: false
            },
        ]
    );
}

#[test]
fn moving_the_active_workspace_restates_exactly_two_handles() {
    assert_eq!(
        between(snapshot(3, 0), snapshot(3, 2)),
        &[
            Restated {
                index: 0,
                active: false
            },
            Restated {
                index: 2,
                active: true
            },
        ]
    );
}

#[test]
fn a_new_workspace_is_created_already_active_rather_than_restated() {
    // Opening a window on the trailing empty workspace: the list grows and
    // the new one is the active one. The handle must carry `active` in its
    // first `state` event -- creating it inactive and then restating it would
    // be a visible flicker in a bar within a single `done` batch.
    assert_eq!(
        between(snapshot(2, 1), snapshot(3, 2)),
        &[
            Restated {
                index: 1,
                active: false
            },
            Added {
                index: 2,
                active: true
            },
        ]
    );
}

#[test]
fn a_removed_active_workspace_is_never_restated_on_its_way_out() {
    // The shape a closing window makes: the workspace that was active is
    // gone, and the active index has moved to one that survives.
    assert_eq!(
        between(snapshot(3, 2), snapshot(2, 0)),
        &[
            Restated {
                index: 0,
                active: true
            },
            Removed { index: 2 },
        ]
    );
}

#[test]
fn shrinking_removes_the_whole_tail_last_one_first() {
    assert_eq!(
        between(snapshot(4, 0), snapshot(1, 0)),
        &[
            Removed { index: 3 },
            Removed { index: 2 },
            Removed { index: 1 },
        ]
    );
}

#[test]
fn growing_without_moving_the_active_workspace_only_adds() {
    assert_eq!(
        between(snapshot(1, 0), snapshot(3, 0)),
        &[
            Added {
                index: 1,
                active: false
            },
            Added {
                index: 2,
                active: false
            },
        ]
    );
}

#[test]
fn a_shrink_that_keeps_the_active_index_only_removes() {
    assert_eq!(
        between(snapshot(3, 1), snapshot(2, 1)),
        &[Removed { index: 2 }]
    );
}

#[test]
fn no_change_ever_names_an_index_outside_the_list_it_belongs_to() {
    // The invariant every executor of these changes depends on: `Restated`
    // and `Removed` address handles the client already has (`published`),
    // `Added` addresses positions the new list has (`current`), and no index
    // is ever named twice.
    let counts = 0..6usize;
    for published_count in counts.clone() {
        for current_count in counts.clone() {
            for published_active in 0..published_count.max(1) {
                for current_active in 0..current_count.max(1) {
                    let published = snapshot(published_count, published_active);
                    let current = snapshot(current_count, current_active);
                    let list = between(published, current);
                    let mut seen: Vec<usize> = Vec::new();
                    for change in &list {
                        let index = match *change {
                            Added { index, .. } => {
                                assert!(
                                    index >= published_count && index < current_count,
                                    "{change:?} for {published:?} -> {current:?}"
                                );
                                index
                            }
                            Restated { index, .. } => {
                                assert!(
                                    index < published_count && index < current_count,
                                    "{change:?} for {published:?} -> {current:?}"
                                );
                                index
                            }
                            Removed { index } => {
                                assert!(
                                    index >= current_count && index < published_count,
                                    "{change:?} for {published:?} -> {current:?}"
                                );
                                index
                            }
                            // `changes` never renames: renames come from the
                            // origins half (`renames`), which these lists do
                            // not cover.
                            Renamed { .. } => {
                                unreachable!("{change:?} for {published:?} -> {current:?}")
                            }
                        };
                        assert!(
                            !seen.contains(&index),
                            "index {index} named twice for {published:?} -> {current:?}"
                        );
                        seen.push(index);
                    }
                    assert_eq!(
                        list.is_empty(),
                        published == current,
                        "for {published:?} -> {current:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn exactly_one_handle_is_left_active_afterwards() {
    // Replays each change list against a model of what a client believes,
    // and checks the client ends up agreeing with the core -- the property
    // the individual cases above are examples of.
    let counts = 1..6usize;
    for published_count in counts.clone() {
        for current_count in counts.clone() {
            for published_active in 0..published_count {
                for current_active in 0..current_count {
                    let published = snapshot(published_count, published_active);
                    let current = snapshot(current_count, current_active);
                    // What the client believes right now: one bool per handle.
                    let mut client: Vec<bool> = (0..published_count)
                        .map(|index| index == published_active)
                        .collect();
                    for change in between(published, current) {
                        match change {
                            Added { index, active } => {
                                assert_eq!(index, client.len(), "handles must stay positional");
                                client.push(active);
                            }
                            Restated { index, active } => client[index] = active,
                            Removed { index } => {
                                assert_eq!(index + 1, client.len(), "removals must be a suffix");
                                client.pop();
                            }
                            // See above: `changes` never renames.
                            Renamed { .. } => {
                                unreachable!("{change:?} for {published:?} -> {current:?}")
                            }
                        }
                    }
                    assert_eq!(client.len(), current_count, "{published:?} -> {current:?}");
                    assert_eq!(
                        client.iter().filter(|active| **active).count(),
                        1,
                        "{published:?} -> {current:?} left {client:?}"
                    );
                    assert!(client[current_active], "{published:?} -> {current:?}");
                }
            }
        }
    }
}

/// The renames between two origin tag lists, as a `Vec`.
fn renamed_between(published: &[Option<u64>], current: &[Option<u64>]) -> Vec<Change> {
    let mut out = Vec::new();
    renames(published, current, &mut out);
    out
}

#[test]
fn identical_tags_rename_nothing() {
    for tags in [
        vec![],
        vec![None],
        vec![None, None, None],
        vec![Some(7)],
        vec![None, Some(7), None],
    ] {
        assert_eq!(renamed_between(&tags, &tags), &[], "{tags:?}");
    }
}

#[test]
fn a_new_tag_renames_only_its_workspace() {
    assert_eq!(
        renamed_between(&[None, None, None], &[None, Some(7), None]),
        &[Renamed { index: 1 }]
    );
}

#[test]
fn a_cleared_tag_renames_only_its_workspace() {
    assert_eq!(
        renamed_between(&[None, Some(7), None], &[None, None, None]),
        &[Renamed { index: 1 }]
    );
}

#[test]
fn a_changed_tag_renames_only_its_workspace() {
    // A chained adoption re-tagging only untagged workspaces never does
    // this (it keeps the earliest origin), but a defensive executor must
    // still rename exactly the workspace that changed.
    assert_eq!(
        renamed_between(&[Some(7)], &[Some(9)]),
        &[Renamed { index: 0 }]
    );
}

#[test]
fn renames_never_reach_past_the_overlap() {
    // Added and removed tails are the count half's business: a longer list
    // with new tags renames nothing (the `Added` carries the name), and a
    // shorter one renames only survivors.
    assert_eq!(renamed_between(&[], &[Some(7), Some(7)]), &[]);
    assert_eq!(renamed_between(&[Some(7), Some(7), None], &[Some(7)]), &[]);
    assert_eq!(
        renamed_between(&[Some(7), None], &[None]),
        &[Renamed { index: 0 }]
    );
}
