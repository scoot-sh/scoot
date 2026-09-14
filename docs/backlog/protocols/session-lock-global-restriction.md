---
title: "The `ext_session_lock_manager_v1` global is offered to every client"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# The `ext_session_lock_manager_v1` global is offered to every client

The `ext_session_lock_manager_v1` global is offered to every client
(item 18). The protocol explicitly allows restricting it ("the compositor
may choose to restrict this protocol to a special client"), and Smithay's
helper takes a client filter flexwm passes `|_| true` to. There is nothing
to filter *on* today — flexwm has no `wp_security_context_v1` support, so
every client is equally privileged — so this belongs with that protocol,
not as a bespoke allow-list.
