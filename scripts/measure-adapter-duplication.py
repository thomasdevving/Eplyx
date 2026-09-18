#!/usr/bin/env python3
"""Measure line-identical overlap between the two adapters, method by method.

Reproduces the research phase's table so Phase U1 has a before/after control.
A line counts when it is neither blank nor a comment. Comparison is multiset
based: a line duplicated N times on one side and M on the other contributes
min(N, M).
"""
import re, sys, collections, os

def strip(lines):
    out = []
    for raw in lines:
        s = raw.strip()
        if not s or s.startswith("//") or s.startswith("#["):
            continue
        out.append(s)
    return out

def methods(path):
    """Extract `fn name(...) { ... }` bodies by brace matching."""
    src = open(path).read().split("\n")
    found = {}
    i = 0
    while i < len(src):
        m = re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+([a-z_0-9]+)", src[i])
        if m:
            name = m.group(1)
            # find opening brace
            depth = 0
            started = False
            body = []
            j = i
            while j < len(src):
                line = src[j]
                # ignore braces inside string literals crudely
                for ch in line:
                    if ch == "{":
                        depth += 1
                        started = True
                    elif ch == "}":
                        depth -= 1
                body.append(line)
                j += 1
                if started and depth <= 0:
                    break
            if name not in found:
                found[name] = body
            i = j
            continue
        i += 1
    return found

def in_tests(path):
    """Line index where `mod tests` starts, so tests are excluded."""
    src = open(path).read().split("\n")
    for n, line in enumerate(src):
        if re.match(r"\s*mod (tests|malformed_input)", line):
            return n
    return len(src)

def methods_prod(path):
    cut = in_tests(path)
    src = open(path).read().split("\n")[:cut]
    tmp = path + ".prod.tmp"
    open(tmp, "w").write("\n".join(src))
    try:
        return methods(tmp)
    finally:
        os.unlink(tmp)

def main(a, b, label):
    ma, mb = methods_prod(a), methods_prod(b)
    shared = sorted(set(ma) & set(mb))
    rows = []
    tot_a = tot_b = tot_i = 0
    for name in shared:
        la, lb = strip(ma[name]), strip(mb[name])
        ca, cb = collections.Counter(la), collections.Counter(lb)
        identical = sum(min(ca[k], cb[k]) for k in ca)
        rows.append((name, len(la), len(lb), identical,
                     (100 * identical // len(lb)) if lb else 0))
        tot_a += len(la); tot_b += len(lb); tot_i += identical
    rows.sort(key=lambda r: -r[4])
    print(f"\n## {label}")
    print(f"{'method':<24}{'A':>6}{'B':>6}{'ident':>7}{'of B':>7}")
    for name, la, lb, ident, pct in rows:
        print(f"{name:<24}{la:>6}{lb:>6}{ident:>7}{pct:>6}%")
    pct = (100 * tot_i // tot_b) if tot_b else 0
    print(f"{'TOTAL':<24}{tot_a:>6}{tot_b:>6}{tot_i:>7}{pct:>6}%")
    return tot_a, tot_b, tot_i, pct

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else "overlap")
