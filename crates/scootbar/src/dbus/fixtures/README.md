# D-Bus wire fixtures

Messages marshalled by a library other than this crate's `Writer`, so
a test of the reader is not a test of a writer and reader that agree
with each other.

`sdbus-getall-call.bin`: one `GetAll` method call whose body is the
`a{sv}` an item's `GetAll` reply carries (an `IconPixmap`, an
`AttentionIconPixmap`, a `ToolTip`, a nested unknown `a{sv}`, a menu
path). Marshalled by sd-bus (`busctl`), captured off a private
`dbus-daemon` with `dbus-monitor --binary`, then cut to the one message
(a message at a time: each starts 8-aligned, as on a real stream):

```sh
dbus-daemon --session --address=unix:path=$PWD/bus.sock --fork
export DBUS_SESSION_BUS_ADDRESS=unix:path=$PWD/bus.sock
dbus-monitor --binary > mon.bin &
busctl --user call org.example.Fixture /Item org.freedesktop.DBus.Properties \
  GetAll "a{sv}" 9 Id s fixture Title s hello Status s Active \
  IconPixmap "a(iiay)" 2  2 2 16 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16  1 1 4 200 100 50 25 \
  AttentionIconPixmap "a(iiay)" 1  1 1 4 9 9 9 9 \
  ToolTip "(sa(iiay)ss)" name 1  1 1 4 7 7 7 7  ttitle ttext \
  Extra "a{sv}" 2  Q "a(xx)" 2 5 6 7 8  Z "(ytd)" 1 2 3.5 \
  ItemIsMenu b true Menu o /MenuBar
```

The call fails (nobody owns the name); the monitor saw it anyway. Split
`mon.bin` into messages by each header's lengths and keep the one with
`GetAll`.
