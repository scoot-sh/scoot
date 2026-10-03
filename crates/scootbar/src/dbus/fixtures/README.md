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

`sdbus-mpris-getall-call.bin` and `sdbus-mpris-changed-signal.bin`: what
the media module reads of a player, marshalled by sd-bus (`busctl`, systemd
261) and captured the same way. The first is a call whose body is the
`a{sv}` a player's `GetAll` answers with (`PlaybackStatus`, a `Metadata`
dictionary with a track id, a length, an art URL, a title with non-ASCII
text, two artists and an album, then `Position`, `Volume`, `Rate` and
three `Can*` flags); the second is a `PropertiesChanged` signal
(`sa{sv}as`: the status, a position, a `Metadata` of a title and one
artist, and `CanGoNext` and `Volume` invalidated):

```sh
busctl --user call org.example.Fixture /org/mpris/MediaPlayer2 \
  org.freedesktop.DBus.Properties GetAll "a{sv}" 8 \
  PlaybackStatus s Playing \
  Metadata "a{sv}" 6 mpris:trackid o /org/mpris/track/1 mpris:length x 215000000 \
    mpris:artUrl s file:///tmp/cover.jpg xesam:title s "Sønġ ♪" \
    xesam:artist as 2 Ada Bo xesam:album s Alb \
  Position x 42000000 Volume d 0.5 CanGoNext b true CanGoPrevious b false \
  CanControl b true Rate d 1.0
busctl --user emit /org/mpris/MediaPlayer2 org.freedesktop.DBus.Properties \
  PropertiesChanged "sa{sv}as" org.mpris.MediaPlayer2.Player 3 \
  PlaybackStatus s Paused Position x 7 \
  Metadata "a{sv}" 2 xesam:title s Second xesam:artist as 1 Cy \
  2 CanGoNext Volume
```

Both are in the dbus fuzz target's seed corpus too.
