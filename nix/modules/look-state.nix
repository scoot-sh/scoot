# One file under the live look's rendered directory at RUNTIME (never
# evaluation): `$SCOOT_LOOK_STATE` names the state dir in a private
# session (headless, or any backend given `--private-look`: the
# compositor exports it to the children it spawns, so binds spawned
# there read their own session's look), else the shared user state
# (`$XDG_STATE_HOME/scoot`, else `~/.local/state/scoot` -- the same rule
# `scoot look` resolves). A function, not a module: takes the leaf name,
# returns a shell expression. Callers run it when they run and fall back
# to whatever they run today when the file is absent, so a session that
# never applied a look keeps byte-identical behavior.
name: "\${SCOOT_LOOK_STATE:-\${XDG_STATE_HOME:-$HOME/.local/state}/scoot}/look/${name}"
