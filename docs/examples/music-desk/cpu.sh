#!/bin/sh
# CPU busy percent every 5 s, from /proc/stat deltas.
read -r _ a b c d e f g h _ < /proc/stat; pt=$((a+b+c+d+e+f+g+h)); pi=$((d+e))
while :; do sleep 5
  read -r _ a b c d e f g h _ < /proc/stat; t=$((a+b+c+d+e+f+g+h)); i=$((d+e))
  dt=$((t-pt)); [ $dt -gt 0 ] && printf "\U000F0EE0 %s%%\n" "$(( (100*(dt-(i-pi))) / dt ))"; pt=$t; pi=$i
done
