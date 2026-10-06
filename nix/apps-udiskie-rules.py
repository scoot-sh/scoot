# What the desktop profile's automounter mounts, judged by udiskie's own
# device matcher (`Mounter._ignore_device`, which `is_handleable`, the
# automount and `udiskie-umount -a` all go through), under the device
# rules its unit's command line selects: argv[1] is that `ExecStart`.
#
# The devices are fakes whose attributes follow udisks 2.11's HintSystem
# rules (`src/udiskslinuxblock.c` `update_hints`: a device is "system"
# unless its Drive is removable or on usb/ieee1394, and a dm-crypt
# cleartext device has no Drive at all, so it is always "system"). That
# last rule is the trap a rule of our own on `is_external` alone falls
# into: it ignores every encrypted USB stick's unlocked filesystem.
import shlex
import sys

from udiskie.config import Config
from udiskie.mount import Mounter


class Dev:
    def __init__(self, name, system, partition=None, cleartext_of=None, loop_file=None):
        self.name = name
        self.is_external = not system
        self.is_systeminternal = system
        self.partition_slave = partition
        self.luks_cleartext_slave = cleartext_of
        self.is_partition = partition is not None
        self.is_luks_cleartext = cleartext_of is not None
        self.is_toplevel = not self.is_partition and not self.is_luks_cleartext
        self.is_loop = loop_file is not None
        self.loop_file = loop_file
        self.is_block = True
        self.is_ignored = False
        self.symlinks = []
        self.device_file = "/dev/" + name
        self.object_path = name


argv = shlex.split(sys.argv[1])
assert argv[0].endswith("/bin/udiskie"), argv
rules = []
if "-c" in argv:
    rules = Config.from_file(argv[argv.index("-c") + 1]).device_config
mounter = Mounter(udisks=None, config=rules)

usb = Dev("sdb", False)
usb_luks = Dev("sdb1", False, partition=usb)
usb_clear = Dev("dm-3", True, cleartext_of=usb_luks)
nvme = Dev("nvme0n1", True)
nvme_other_os = Dev("nvme0n1p3", True, partition=nvme)
nvme_luks = Dev("nvme0n1p4", True, partition=nvme)
nvme_clear = Dev("dm-0", True, cleartext_of=nvme_luks)
image = Dev("loop0", False, loop_file="/home/u/disk.img")

want = {
    # A USB stick, its LUKS container, and the container's unlocked
    # filesystem: all mounted (the last is what a passphrase is for).
    usb: False,
    usb_luks: False,
    usb_clear: False,
    # The machine's own disk, another OS's partition on it (a dual-boot
    # Windows or macOS volume), and an internal LUKS volume both locked
    # and unlocked: all left alone (no admin prompt at login).
    nvme: True,
    nvme_other_os: True,
    nvme_luks: True,
    nvme_clear: True,
    # A disk image the user attached (`udisksctl loop-setup`): mounted.
    image: False,
}
got = {d: mounter._ignore_device(d) for d in want}
bad = [f"{d.name}: ignored={got[d]}, want {want[d]}" for d in want if got[d] != want[d]]
assert not bad, "; ".join(bad)
