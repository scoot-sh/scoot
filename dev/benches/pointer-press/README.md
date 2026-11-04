# Pointer press cost

What a button press pays for `State::dismiss_launcher_on_outside_press` (the
launcher click-outside check), and what a whole press and release costs for
scale. The bench is `pointer_press_cost`
(`crates/scoot/src/compositor/headless/bench/pointer.rs`), an `#[ignore]`d
test that prints and asserts nothing; compare a later change against the
numbers below by hand.

```sh
cargo test --release -p scoot --bin scoot pointer_press_cost -- --ignored --nocapture --test-threads=1
```

Scene: three client windows, the last floated, the pointer over it, no
launcher mapped (the common case: the check returns at once). 5 runs of
20000 presses each; the figure is the median per press.

| Measurement | Median | Min | Max |
|---|---|---|---|
| launcher check alone, no launcher | 171 ns | 164 ns | 204 ns |
| press + release over a window | 4.434 us | 4.151 us | 5.183 us |

Machine: the scoot dev VM (6 vCPUs, `nproc` = 6), release build, 2026-11-04,
tree at `origin/main` 737cec6cf (the bench and the check are unchanged since
#803). The check alone is the call's whole cost, since it is the only thing
#803 added to a press.

#803's own run, recorded only in its PR body, on the same VM: 190 ns for the
check alone, 4.49 us for press + release. Run to run the two agree within
about 10%.
