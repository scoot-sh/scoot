#!/bin/sh
while :; do printf "\U000F061A %s\n" "$(cut -d" " -f1 /proc/loadavg)"; sleep 10; done
