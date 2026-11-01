#!/usr/bin/env python3
"""Compiles open filter lists into Zajil for Android's compact host lists.

Usage: compile.py <name> <out.txt> <source-file>...

Each source is either an Adblock Plus list (EasyList, EasyPrivacy, AdGuard)
or a hosts file ("0.0.0.0 host"). Only whole-host rules are kept: a rule
«||host^» with no path and no «domain=» limit blocks <host>; an exception
«@@||host^» keeps it (written as «@@host»). Everything else (paths, element
hiding, scripts) is left out: the app matches hosts only, fast and on the
phone (ADR-0012, section 17). The output is one host per line, sorted, with
a header naming the sources; nothing in it runs.
"""
import os
import re
import sys

HOST = re.compile(r"^(?=.{1,253}$)([a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)(?:\.([a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?))+$")
# Options that limit where a rule applies: such a rule is not a whole-host one.
LIMITED = ("domain=", "denyallow=", "to=", "from=", "header=", "method=", "redirect", "removeparam", "csp", "permissions", "replace", "badfilter", "~")


def rules(text):
    blocked, allowed = set(), set()
    for line in text.splitlines():
        line = line.strip()
        if not line or line[0] in "!#[":
            # A hosts file: «0.0.0.0 host».
            continue
        m = re.match(r"^(?:0\.0\.0\.0|127\.0\.0\.1)\s+(\S+)$", line)
        if m:
            host = m.group(1).lower().rstrip(".")
            if HOST.match(host) and host not in ("localhost",):
                blocked.add(host)
            continue
        exception = line.startswith("@@")
        rule = line[2:] if exception else line
        m = re.match(r"^\|\|([a-z0-9.-]+)\^(?:\$(.*))?$", rule, re.I)
        if not m:
            continue
        host, options = m.group(1).lower().rstrip("."), (m.group(2) or "").lower()
        if any(o.strip().startswith(LIMITED) for o in options.split(",") if o):
            continue
        if not HOST.match(host):
            continue
        (allowed if exception else blocked).add(host)
    return blocked, allowed


def credits(name):
    """The sources and licences of list <name>, from sources.txt beside this file."""
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "sources.txt")
    with open(path, encoding="utf-8") as f:
        for line in f:
            parts = line.split(None, 2)
            if len(parts) == 3 and parts[0] == name and not line.startswith("#"):
                yield f"# {parts[1]} ({parts[2].strip()})"


def main():
    name, out, sources = sys.argv[1], sys.argv[2], sys.argv[3:]
    blocked, allowed = set(), set()
    for path in sources:
        with open(path, encoding="utf-8", errors="replace") as f:
            b, a = rules(f.read())
        blocked |= b
        allowed |= a
    blocked -= allowed
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write(f"# zajil-hosts-1 {name}\n")
        # Attribution, as the lists' licences ask: compiled from these, under their terms.
        f.write("# Compiled to hosts by Zajil (tools/blocklists/compile.py) from:\n")
        for credit in credits(name):
            f.write(credit + "\n")
        for host in sorted(blocked):
            f.write(host + "\n")
        for host in sorted(allowed):
            f.write("@@" + host + "\n")
    print(f"{name}: {len(blocked)} hosts blocked, {len(allowed)} kept")


if __name__ == "__main__":
    main()
