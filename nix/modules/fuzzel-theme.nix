# The look's roles as fuzzel CLI colors (opaque: fuzzel takes
# `rrggbbaa`), shared by the clipboard history picker and the launcher:
# both theme the same seven leaves from the same roles, so they read as
# one menu wherever they open. A function, not a module: takes `lib`,
# returns `look: flags` (the leading space is inside, so call sites read
# plainly). Callers gate it on their own theme target (`optionalString
# themed ...`): nothing without a look (or opted out), where fuzzel's
# own style stands and a value the user set in fuzzel's own config wins
# per key for everything these flags leave alone (font, lines, ...).
{ lib }:
look:
let
  hexA = color: "${lib.removePrefix "#" color}ff";
in
" --background-color=${hexA look.barColors.background}"
+ " --text-color=${hexA look.barColors.foreground}"
+ " --border-color=${hexA look.appearance.focus_ring_active_color}"
+ " --selection-color=${hexA look.barColors.accent}"
+ " --selection-text-color=${hexA look.barColors.background}"
+ " --match-color=${hexA (look.barColors.hover or look.barColors.accent)}"
+ " --prompt-color=${hexA look.barColors.foreground}"
