# M6 bluetooth on the dev VM

Module-level cost of the bluetooth module ([the entry](../../backlog/lightest.md#m6-bluetooth-module-level-cost-measured-2026-10-03)):
one 60 s idle window per row, release builds, a headless `scoot`, a private
`dbus-daemon` as the system bus, and BlueZ a scripted peer.

| Script | What it is |
|---|---|
| `scripts/runall.sh` | the rows of the cost table (release binaries in `/tmp/bt-bins`: `scootbar-main`, `scootbar-off`, `scootbar-on`) |
| `scripts/measure.sh` | one row: one scoot, one daemon (none for `nobus`), one bar, the mode's BlueZ; samples `VmRSS`, `Pss`, context switches and fds at 14 s and 74 s |
| `scripts/bluez.pl` | the BlueZ: owns `org.bluez`, answers `GetManagedObjects` and `GetAll` from `fixtures/`, prints `Set(Powered)`, `flood N` blasts N `PropertiesChanged`. Raw-socket Perl (an independent marshaller), written against `src/dbus/bluez/build.rs`'s shapes and verified with `busctl` as a second client |
| `scripts/fixtures/` | the managed set (one adapter on, one device connected with 72% battery) and the per-interface `GetAll` answers, marshalled by the module's own test builders |

`logs/` holds the raw per-row output this entry's table was read from
(`measure-final.log`).
