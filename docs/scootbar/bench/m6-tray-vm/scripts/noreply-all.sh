cd /tmp/tray-takeover-run
D=/tmp/tray-takeover-run/nr; rm -rf $D; mkdir -p $D
CONF=/nix/store/pizr05f9y1y76nh65al07p469lkbb96z-dbus-1.16.2/share/dbus-1/session.conf
sed 's|</busconfig>|<limit name="reply_timeout">5000</limit></busconfig>|' $CONF > $D/short.conf
# 1. dbus-daemon with reply_timeout 5 s configured, client timeout 60 s
dbus-daemon --config-file=$D/short.conf --nofork --address=unix:path=$D/short.sock >/dev/null 2>&1 &
S1=$!
# 2. dbus-daemon with the stock session.conf, client timeout 400 s
dbus-daemon --config-file=$CONF --nofork --address=unix:path=$D/stock.sock >/dev/null 2>&1 &
S2=$!
sleep 1
bash noreply.sh short-conf-daemon unix:path=$D/short.sock 60000 &
bash noreply.sh stock-conf-daemon unix:path=$D/stock.sock 400000 &
# 3. the VM's user dbus-broker, client timeout 130 s
bash noreply.sh user-dbus-broker unix:path=/run/user/1000/bus 130000 &
wait %3 %4 %5 2>/dev/null
wait
kill $S1 $S2 2>/dev/null
echo FINISHED > $D/done
