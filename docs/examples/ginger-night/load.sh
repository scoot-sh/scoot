#!/bin/sh
# The bar's `load` module draws its icon (bar.toml's `icon` key); this prints
# the value alone.
while :; do cut -d" " -f1 /proc/loadavg; sleep 10; done
