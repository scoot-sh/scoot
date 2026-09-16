---
title: "Nothing bounds how many `ext_workspace_manager_v1` objects one client may bind."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Nothing bounds how many `ext_workspace_manager_v1` objects one client may bind.

Nothing bounds how many `ext_workspace_manager_v1` objects one client
may bind. Each costs a registry entry and a handle per workspace, and
every workspace change walks them all. Not specific to this protocol —
the same is true of layer surfaces, `wl_shm` pools (item 7's cap is
per-pool, and its own entry already asks for per-client accounting) and
IPC connections (see the screenshot/connection-cap entry below) — but
worth naming while the memory of writing it is fresh: the fix belongs
with those, as per-client accounting, not as a one-off limit here.

Update 2026-09-16: `ext_foreign_toplevel_list_v1` (see
`resolved/foreign-toplevel-list-done.md`) has exactly the same shape and
is deliberately left to this entry rather than capped on its own — each
bind costs one handle object per *window*, and every window change walks
every bound list. Whatever per-client accounting closes this should cover
both globals — but size it against the more dangerous of the two
multipliers, not the average: `ext-workspace`'s per-bind cost is the
workspace *count*, which grows only through user-driven layout actions
(`adopt`/`tidy`), so a client cannot inflate it on its own. Foreign-
toplevel's is the window *count*, which one client can create directly and
without limit (see `windows_opened_and_destroyed_at_full_rate_leave_
nothing_behind`'s 200-in-a-burst case) — so a single client can force
`binds × self-created windows` of server-side object allocation, both
factors entirely under that one client's control.

Update 2026-09-16 (PR #49): `zwlr_output_manager_v1` is the third of this
shape. Its per-bind cost is one head object plus one mode object per known
mode — bounded, since flexwm has exactly one output and `Output::modes`
only grows at most once per process (see `output_management.rs`'s module
doc) — so on its own it is the least dangerous of the three multipliers,
closer to `ext-workspace`'s than to foreign-toplevel's. Still the same
fix when one lands: per-client accounting across all of them, not a
one-off limit on any single global.

Update 2026-09-16 (PR #50): `zwlr_foreign_toplevel_manager_v1` (see
`resolved/wlr-foreign-toplevel-management-done.md`) is the fourth, and it
shares the *worst* multiplier rather than adding a new one: its per-bind
cost is one handle object per **window**, exactly like
`ext_foreign_toplevel_list_v1`'s, and window count is the one factor a
single client can inflate on its own and without limit. Two consequences
for whoever sizes the eventual cap:

- **A client now has two globals carrying that multiplier, not one.** The
  worst case one connection can force is `(ext binds + wlr binds) ×
  self-created windows` of server-side object allocation. Nothing about the
  fix changes — per-client accounting across every global of this shape —
  but the budget has to be shared across them, or a client simply spends it
  twice.
- **Its worst walk is `wl_output` binding, not window churn.**
  `wlr_toplevel_output_bound` runs once per `wl_output` bind *by any client*
  and visits every handle of every window — `binds × windows` of work that
  the client provoking it need not own a single window or handle to trigger,
  which none of the other three globals has an equivalent of. It is a plain
  id comparison per handle and sends nothing to the clients it skips, so it
  is cheap per unit; it is the *shape* that belongs in this entry.
  (`refresh_wlr_activation`, on a focus change, is **not** the expensive one
  and an earlier draft of this paragraph said it was: its handle loop is
  inside the changed-bit branch, so a focus change reaches the handles of at
  most two windows, not all of them.)
