#!/usr/bin/env python3
"""Line coverage per workspace crate, as a Markdown table (docs/DEEP_TESTING.md D1).

Reads `cargo llvm-cov report --json --summary-only` and prints one row per
crate under crates/, the most uncovered lines first: where a later phase
should look. A report, never a gate.

Usage: scripts/coverage-by-crate.py COVERAGE_JSON
"""

import json
import re
import sys
from collections import defaultdict


def main(path):
    report = json.load(open(path))
    crates = defaultdict(lambda: [0, 0])
    for data in report["data"]:
        for f in data["files"]:
            m = re.search(r"/crates/([^/]+)/", f["filename"])
            if not m:
                continue
            lines = f["summary"]["lines"]
            crates[m.group(1)][0] += lines["covered"]
            crates[m.group(1)][1] += lines["count"]

    covered = sum(c for c, _ in crates.values())
    total = sum(t for _, t in crates.values())
    print(f"**Workspace:** {covered} of {total} lines, {100 * covered / max(total, 1):.1f}%\n")
    print("| Crate | Lines | Covered | Uncovered |")
    print("|---|---:|---:|---:|")
    for name, (c, t) in sorted(crates.items(), key=lambda kv: kv[1][0] - kv[1][1]):
        print(f"| {name} | {t} | {100 * c / max(t, 1):.1f}% | {t - c} |")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
