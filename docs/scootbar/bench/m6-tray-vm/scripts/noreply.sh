#!/bin/bash
# usage: noreply.sh LABEL ADDRESS CLIENT_TIMEOUT_MS
PY=/nix/store/k8ac8wkni66rngy6xfrf37gj7b9dj7hp-python3-3.14.7-env/bin/python3
export DBUS_SESSION_BUS_ADDRESS=$2
$PY /tmp/tray-takeover-run/noreply.py > /tmp/tray-takeover-run/noreply-$1.owner 2>&1 &
OWNER=$!
sleep 2
start=$(date +%s.%N)
dbus-send --session --print-reply --reply-timeout=$3 --dest=sh.scoot.NoReply /x sh.scoot.NoReply.M > /tmp/tray-takeover-run/noreply-$1.out 2>&1
end=$(date +%s.%N)
echo "$1: dbus-send returned after $(awk "BEGIN{print $end - $start}") s: $(tr '\n' ' ' < /tmp/tray-takeover-run/noreply-$1.out | cut -c1-260)" | tee /tmp/tray-takeover-run/noreply-$1.result
kill $OWNER 2>/dev/null
