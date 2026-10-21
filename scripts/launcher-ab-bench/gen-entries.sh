#!/usr/bin/env bash
# Generates deterministic synthetic .desktop entry dirs for the
# scootlaunch-vs-fuzzel A/B bench (dev/benchmarks/launcher-ab-2026-10.md).
#
# Usage: gen-entries.sh OUTDIR [100 [1000 [5000]]]
# Makes $OUTDIR/<n>/applications/bench-<i>.desktop for each n, with
# realistic names, generic names, comments and keywords, and NO Icon
# lines (both launchers then skip icon lookup: the bench measures
# index/parse/rank/draw, not theme probing). Exec points at /bin/true
# so nothing launchable-or-dangerous hides behind an id; TryExec is
# absent so every file indexes. Output is deterministic: the same
# OUTDIR and n always yields the same bytes (no timestamps in the
# files). One awk process writes all files: no python is required on
# the target.
set -eu
export LC_ALL=C

OUTDIR=${1:-./bench-data}
shift || true
if [ "$#" -eq 0 ]; then
    set -- 100 1000 5000
fi
for n in "$@"; do
    case "$n" in
        ''|*[!0-9]*) echo "gen-entries.sh: not a count: $n" >&2; exit 2 ;;
    esac
    mkdir -p "$OUTDIR/$n/applications"
    awk -v n="$n" -v dir="$OUTDIR/$n/applications" '
    BEGIN {
        names[1]="Firefox|Web Browser|Browse the web|web;browser;internet"
        names[2]="Terminal|Terminal Emulator|Run shell commands|shell;console;terminal"
        names[3]="Files|File Manager|Browse files and folders|files;folders;manager"
        names[4]="Text Editor|Plain-text Editor|Edit text files|text;edit;notes"
        names[5]="Calculator|Calculator|Do arithmetic|math;calc;arithmetic"
        names[6]="Music Player|Audio Player|Play music collections|music;audio;songs"
        names[7]="Video Player|Media Player|Watch video files|video;movie;media"
        names[8]="Photo Viewer|Image Viewer|Look at pictures|photo;image;picture"
        names[9]="Mail Client|Email Client|Read and send mail|mail;email;inbox"
        names[10]="Calendar|Calendar|See dates and events|calendar;date;events"
        names[11]="Settings|System Settings|Change system options|settings;config;system"
        names[12]="Passwords|Password Manager|Keep secrets safe|password;secret;login"
        names[13]="Disk Usage|Disk Analyzer|See what fills the disk|disk;space;usage"
        names[14]="System Monitor|Task Manager|Watch running processes|process;cpu;memory"
        names[15]="Archive Manager|Compression Tool|Open zip and tar files|zip;tar;archive"
        names[16]="Document Viewer|PDF Reader|Read documents|pdf;document;read"
        names[17]="Maps|Map Viewer|Find places and routes|map;places;routes"
        names[18]="Weather|Weather Forecast|See the forecast|weather;forecast;rain"
        names[19]="Notes|Sticky Notes|Jot quick notes|notes;memo;sticky"
        count = 19
        for (i = 1; i <= n; i++) {
            split(names[((i - 1) % count) + 1], f, "|")
            name = f[1]
            if (i > count) { name = name " " i }
            id = sprintf("bench-%05d.desktop", i)
            out = dir "/" id
            print "[Desktop Entry]" > out
            print "Type=Application" >> out
            print "Name=" name >> out
            print "GenericName=" f[2] >> out
            print "Comment=" f[3] >> out
            print "Keywords=" f[4] ";" >> out
            print "Exec=/bin/true bench-" i >> out
            print "Terminal=false" >> out
            close(out)
        }
    }'
    echo "gen-entries.sh: $OUTDIR/$n: $(ls "$OUTDIR/$n/applications" | wc -l) entries"
done
