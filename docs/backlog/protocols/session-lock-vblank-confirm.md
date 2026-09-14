---
title: "`locked` is sent once a blanked frame has been *rendered*, not once a vblank has confirmed it"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# `locked` is sent once a blanked frame has been *rendered*, not once a vblank has confirmed it

`locked` is sent once a blanked frame has been *rendered*, not once a
vblank has confirmed it (item 18, restated precisely in round two after
review traced the original wording against the code and found it too
strong). `confirm_lock` fires on `drew_a_frame`, which `headless.rs` sets
when `render_output` succeeds on the pixman image — before, and independent
of, presentation. `Tty::present` then early-returns without copying anything
or asking for a flip whenever `!self.active` (session paused / VT switched
away) or `self.flip_pending`, and the code's own comment calls that second
skip "an ordinary, frequent, harmless throttle". So the real guarantee is
"rendered, and handed to the presenter if the presenter could take it": up
to one further vblank of the previous — possibly unlocked — frame can stay
on scanout after the client has been told `locked`. Closing it means
confirming from the DRM vblank handler (`tty/mod.rs`'s `DrmEvent::VBlank`),
which is more than it sounds: because `present` skips while a flip is in
flight, the next completion is for the *previous* frame, so this needs to
track which in-flight flip actually carries the blanked frame — and it must
not leave a locker waiting forever for a vblank that cannot arrive while the
session is switched away. Touches the presentation path, so it stays its own
item rather than riding along with a fix round.
