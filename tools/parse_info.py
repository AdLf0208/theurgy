#!/usr/bin/env python3
"""Parse one `/theurgy info` line into shell assignments for `eval`.

The line is printed by `ports/theurgy/mixins/theurgy/support.rs::readout`:

    Calcination Oven at [202, 102, 0]: lit=true, input minecraft:blackstone x2,
      output theurgy:alchemical_salt_strata x1, progress 0/100, heatCache 1, t=1234

    Pyromantic Brazier at [202, 101, 0]: lit=true, fuel empty, remaining 1598 ticks, t=1234

Every field is always printed, empty when the line does not carry it, so a
caller's earlier value can never leak into a later assertion. `ok=1` when the
line is one of ours.
"""
import re
import shlex
import sys

# Every field, and the pattern that finds it (a missing one stays empty).
PATTERNS = {
    "lit": r"lit=(\w+)",
    "input": r"input ([a-z0-9_:]+) x(\d+)",
    "output": r"output ([a-z0-9_:]+) x(\d+)",
    "progress": r"progress (\d+)/(\d+)",
    "heat": r"heatCache (\d+)",
    "fuel": r"fuel ([a-z0-9_:]+) x(\d+)",
    "remaining": r"remaining (\d+) ticks",
    "t": r" t=(\d+)",
}
FIELDS = ["ok", "station", "lit", "input", "input_count", "output", "output_count",
          "progress", "total", "heat", "fuel", "fuel_count", "remaining", "t"]


def main():
    line = sys.stdin.read().strip()
    out = dict.fromkeys(FIELDS, "")
    out["ok"] = "0"
    if line.startswith("Calcination Oven at "):
        out["station"] = "oven"
    elif line.startswith("Pyromantic Brazier at "):
        out["station"] = "brazier"
    else:
        print("ok=0")
        return
    out["ok"] = "1"
    for name, pattern in PATTERNS.items():
        match = re.search(pattern, line)
        if not match:
            continue
        groups = match.groups()
        if name in ("input", "output", "fuel"):
            out[name] = groups[0]
            out[f"{name}_count"] = groups[1]
        elif name == "progress":
            out["progress"] = groups[0]
            out["total"] = groups[1]
        else:
            out[name] = groups[0]
    for key in FIELDS:
        print(f"{key}={shlex.quote(out[key])}")


if __name__ == "__main__":
    main()
