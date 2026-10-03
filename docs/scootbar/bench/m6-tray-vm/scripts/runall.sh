cd /tmp/tray-takeover-run
printf 'right = ["clock"]\n' > clockonly.toml
printf 'left = ["tray"]\n' > trayonly.toml
printf 'left = ["tray"]\nright = ["clock"]\n' > tray.toml
printf 'left = []\ncenter = []\nright = []\n' > none.toml
{
echo "# $(date -u +%FT%TZ) code SHA 6a2662cd7 tree crates=7bc1a04525c8; scoot = /var/cargo-target/release/scoot (existing release build, read-only use); VM load: $(cat /proc/loadavg)"
for spec in "main-none:scootbar-main:none.toml:0:yes" "tray-none:scootbar-tray:none.toml:0:yes" "main-clock:scootbar-main:clockonly.toml:0:yes" "off-clock:scootbar-notray:clockonly.toml:0:yes" "tray-unplaced-clock:scootbar-tray:clockonly.toml:0:yes" "tray-only-nobus:scootbar-tray:trayonly.toml:0:no" "tray-only-bus-0items:scootbar-tray:trayonly.toml:0:yes" "tray-only-bus-1item:scootbar-tray:trayonly.toml:1:yes" "tray-only-bus-8items:scootbar-tray:trayonly.toml:8:yes" "tray-clock-bus-1item:scootbar-tray:tray.toml:1:yes"; do
  IFS=: read label bin cfg items bus <<< "$spec"
  echo "=== $label (bin=$bin config=$cfg items=$items bus=$bus) load=$(cut -d' ' -f1-3 /proc/loadavg)"
  bash measure.sh $label /tmp/tray-takeover-run/$bin /tmp/tray-takeover-run/$cfg $items $bus 2>&1
done
echo FINISHED
} > /tmp/tray-takeover-run/measure-final.log 2>&1
