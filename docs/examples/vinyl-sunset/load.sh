#!/usr/bin/env bash
# bash: its printf expands the \U glyph escapes (dash, Debian's /bin/sh, does not).
while :; do printf "\U000F061A %s\n" "$(cut -d" " -f1 /proc/loadavg)"; sleep 10; done
