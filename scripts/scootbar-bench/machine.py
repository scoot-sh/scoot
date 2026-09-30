"""What the machine was doing around a run: the readings that say whether a
number was taken cool and on mains power, or throttled.

A fanless laptop throttles under load and its clocks move with its power
state, so a run records, before and after each bar's startup rounds and
idle window (``state()``): per CPU the governor and the cpufreq current,
policy-maximum and hardware-maximum frequencies (a thermal cap shows as the
policy maximum falling below the hardware one), every hwmon temperature the
kernel exposes, the power supplies (mains online, battery status and
charge) and the load average. Everything is optional: a reading the kernel
does not offer is left out, never guessed (the Apple M2 exposes no CPU
temperature, only the NAND, battery, charger and radio ones).
"""

import glob
import os


def read(path):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return None


def number(path):
    text = read(path)
    try:
        return int(text)
    except (TypeError, ValueError):
        return None


def cpus():
    """Per CPU: governor and frequencies in kHz."""
    out = {}
    for policy in sorted(glob.glob("/sys/devices/system/cpu/cpu[0-9]*/cpufreq"),
                         key=lambda p: int(p.split("/")[-2][3:])):
        name = policy.split("/")[-2]
        out[name] = {
            "governor": read(f"{policy}/scaling_governor"),
            "cur_khz": number(f"{policy}/scaling_cur_freq"),
            "policy_max_khz": number(f"{policy}/scaling_max_freq"),
            "hw_max_khz": number(f"{policy}/cpuinfo_max_freq"),
        }
    return out


def temperatures():
    """Every hwmon temperature, in degrees Celsius, by ``chip/label``."""
    out = {}
    for chip in sorted(glob.glob("/sys/class/hwmon/hwmon*")):
        name = read(f"{chip}/name") or os.path.basename(chip)
        for path in sorted(glob.glob(f"{chip}/temp*_input")):
            milli = number(path)
            if milli is None:
                continue
            label = read(path.replace("_input", "_label")) or os.path.basename(path)[:-6]
            out[f"{name}/{label}"] = milli / 1000
    return out


def power():
    out = {}
    for supply in sorted(glob.glob("/sys/class/power_supply/*")):
        entry = {k: read(f"{supply}/{k}") for k in ("type", "online", "status", "capacity")}
        out[os.path.basename(supply)] = {k: v for k, v in entry.items() if v is not None}
    return out


def state():
    return {"cpus": cpus(), "temps_c": temperatures(), "power": power(),
            "loadavg": os.getloadavg()}


def summary(state):
    """One line for the console: the highest current frequency, whether any
    policy maximum is below its hardware one, and the hottest reading."""
    cpus = state["cpus"].values()
    cur = max((c["cur_khz"] or 0 for c in cpus), default=0)
    capped = any(c["policy_max_khz"] and c["hw_max_khz"] and c["policy_max_khz"] < c["hw_max_khz"]
                 for c in cpus)
    hottest = max(state["temps_c"].items(), key=lambda kv: kv[1], default=None)
    return (f"max cur {cur / 1000:.0f} MHz, cap {'YES' if capped else 'no'}"
            + (f", hottest {hottest[0]} {hottest[1]:.1f} C" if hottest else ""))
