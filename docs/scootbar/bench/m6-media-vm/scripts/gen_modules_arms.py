# NOTE: a record of how the `modules!` arms of src/cli.rs were regenerated
# (the tray made them 64, the media module 128). It hardcodes the seven
# optional modules of that day (volume, microphone, network, brightness,
# battery, tray, media), needs a python3 the dev shell does not carry, and
# does not apply to `popup`, which is a feature and not a module (the arms
# were unaffected by it). Edit the `cfgorder` list to the modules of the
# day, or write the arms by hand. The unit test
# `cli::tests::the_modules_line_is_the_registry_in_order` checks every build's
# line against the registry, so a mistake in an arm shows in the feature matrix.
# The regex below consumes the `// One arm per combination...` comment above the
# arms, so the script writes it back (the first run of it did not, and the
# comment was restored by hand). The `media_head!`/`media_tail!` macros and their uses in the other `modules!`
# arms were written by hand. Regenerates the 128 `modules!` arms of crates/scootbar/src/cli.rs (builds with
# no clock, workspaces or window-title: one arm per subset of the seven optional
# modules, alphabetical in the listing). Usage: python3 gen_modules_arms.py < cli.rs > cli.new.rs
import re,sys
s=sys.stdin.read()
blk=re.compile(r'(?:// [^\n]*\n)*#\[cfg\(all\(\n    not\(feature = "clock"\),\n    not\(feature = "workspaces"\),\n    not\(feature = "window-title"\),\n.*?\n\)\)\]\nmacro_rules! modules \{\n.*?\n    \};\n\}\n',re.S)
ms=list(blk.finditer(s))
start,end=ms[0].start(),ms[-1].end()
cfgorder=['volume','microphone','network','brightness','battery','tray','media']
names=sorted(cfgorder)
out=[]
for mask in range(1<<len(cfgorder)):
    on={f for i,f in enumerate(cfgorder) if mask>>i&1}
    conds=['    not(feature = "clock"),','    not(feature = "workspaces"),','    not(feature = "window-title"),']
    for f in cfgorder:
        conds.append(('    feature = "%s"'%f) if f in on else ('    not(feature = "%s")'%f))
    body=",\n".join(c.rstrip(',') for c in conds)
    mods=[n for n in names if n in on]
    if mods:
        mid='     --center IDS         comma-separated, in order. Giving any of the three\n     --right IDS          sets the whole layout. Modules: %s\n'%", ".join(mods)
    else:
        mid='     --center IDS         comma-separated, in order. This build has none, so\n     --right IDS          only an empty list is taken\n'
    out.append('#[cfg(all(\n%s\n))]\nmacro_rules! modules {\n    () => {\n        "    --left IDS           the modules along the left, center and right,\n%s     --padding N          logical pixels either side of each module, 0 to 1024\n                            (default 8)\n     --spacing N          logical pixels between neighbouring modules, 0 to\n                            1024 (default 0)\n"\n    };\n}\n'%(body,mid))
comment = "// One arm per combination of the modules a build may leave out, with no\n// clock, workspaces or window title to end the list: generated, in\n// alphabetical order of the names listed.\n"
sys.stdout.write(s[:start]+comment+"".join(out)+s[end:])
