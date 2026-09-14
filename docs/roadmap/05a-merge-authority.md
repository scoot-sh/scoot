---
item: "5a"
title: "Merge authority history"
status: "done"
area: "meta"
pr: null
commit: null
---

# Merge authority history

**5a. Merge authority history.** `gh pr merge` was denied by the
auto-mode classifier ("Merge Without Review") when attempted autonomously
on PR #8 — a harness-level gate. The user approved merging PR #8 in chat;
for PR #9 that approval didn't carry forward and a fresh check-in was
asked for and given. After that, the user made it standing policy (see
`CLAUDE.md`): once `flexwm-reviewer` has reported back and the
coordinating session has actually weighed the diagnosis, merge without
asking again — first exercised merging PR #9 and #10 together.
