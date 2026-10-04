#!/bin/sh
# CPU busy percent every 5 s, from /proc/stat deltas. The bar's `cpu` module
# draws its icon (bar.toml's `icon` key); this prints the value alone.
read -r _ a b c d e f g h _ < /proc/stat; pt=$((a+b+c+d+e+f+g+h)); pi=$((d+e))
while :; do sleep 5
  read -r _ a b c d e f g h _ < /proc/stat; t=$((a+b+c+d+e+f+g+h)); i=$((d+e))
  dt=$((t-pt)); [ $dt -gt 0 ] && printf "%s%%\n" "$(( (100*(dt-(i-pi))) / dt ))"; pt=$t; pi=$i
done
