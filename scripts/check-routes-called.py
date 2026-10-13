#!/usr/bin/env python3
"""Every operation in the contract is called by a test (docs/DEEP_TESTING.md D1).

Reads the route logs a test run wrote (`FCP_ROUTE_LOG`, written by
`fcp_api_core::route_log`) and compares them with `docs/openapi.json`. An
operation counts as called when some request reached it and was not refused
before its handler: 401, 403, 405 and 429 do not count.

Fails when:
  - an operation no test calls is missing from the allowlist;
  - an allowlisted operation is called (take it off the list);
  - an allowlisted operation is not in the contract.

The allowlist, scripts/routes-uncalled.txt, is one operation per line,
`METHOD /api/v1/path  reason`, with `#` comments. Every entry names why.

Usage: scripts/check-routes-called.py LOG [LOG...]
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTRACT = ROOT / "docs" / "openapi.json"
ALLOWLIST = ROOT / "scripts" / "routes-uncalled.txt"
METHODS = ("get", "put", "post", "delete", "patch")
REFUSED = {401, 403, 405, 429}


def shape(path):
    """A path template with its parameters unnamed: axum's `:id`/`*rest`
    and the contract's `{id}` compare equal."""
    return re.sub(r"\{[^}]*\}|:[^/]+|\*[^/]+", "{}", path)


def main(logs):
    if not logs:
        sys.exit(__doc__)
    spec = json.loads(CONTRACT.read_text())
    contract = {}
    for path, item in spec["paths"].items():
        for method in METHODS:
            if method in item:
                contract[(method.upper(), shape(path))] = f"{method.upper()} {path}"

    called, reached, lines = set(), set(), 0
    for log in logs:
        for line in Path(log).read_text().splitlines():
            method, template, status = line.split()
            lines += 1
            key = (method, shape(template))
            reached.add(key)
            if int(status) not in REFUSED:
                called.add(key)
    if lines < 100:
        sys.exit(f"only {lines} lines in the route logs; were they written?")

    allowed = {}
    for n, line in enumerate(ALLOWLIST.read_text().splitlines(), 1):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split(None, 2)
        if len(parts) < 3:
            sys.exit(f"{ALLOWLIST.name}:{n}: `METHOD PATH reason`, the reason too")
        allowed[(parts[0].upper(), shape(parts[1]))] = f"{parts[0].upper()} {parts[1]}"

    errors = []
    for key, name in sorted(contract.items(), key=lambda kv: kv[1]):
        if key not in called and key not in allowed:
            how = " (only refused)" if key in reached else ""
            errors.append(f"never called{how}: {name}")
    for key, name in sorted(allowed.items(), key=lambda kv: kv[1]):
        if key not in contract:
            errors.append(f"allowlisted but not in the contract: {name}")
        elif key in called:
            errors.append(f"allowlisted but called, take it off the list: {name}")

    outside = sorted(f"{m} {p}" for m, p in called - contract.keys() if p.startswith("/api/v1"))
    for name in outside:
        print(f"called, not in the contract: {name}")

    done = len(contract.keys() & called)
    print(f"{done} of {len(contract)} operations called; {len(allowed)} allowlisted")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main(sys.argv[1:])
