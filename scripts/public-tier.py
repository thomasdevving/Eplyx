#!/usr/bin/env python3
"""Run the part of the test suite a clean checkout can run, and nothing less.

Some fixtures are kept out of git on purpose: historical captures that carry
provider-origin fields (see fixtures/lifecycle/README.md and .gitignore). They
are imported locally, and the tests that read them fail without them by design.
GitHub CI runs on a clean checkout, so it runs the *public tier*: every target
and test except the ones recorded in scripts/private-fixture-tests.json. The full
suite, private payloads included, is `make regression`.

Nothing is skipped silently:
  * the exclusions are a committed, generated file, printed in every summary;
  * an exclusion naming a target or test that no longer exists fails the run;
  * a new test that needs a private payload fails the public tier until it is
    recorded, because nothing outside the file is ever skipped.

  scripts/public-tier.py test             cargo test, public tier (CI)
  scripts/public-tier.py clippy           cargo clippy -D warnings, public tier (CI)
  scripts/public-tier.py check            validate the exclusions against cargo metadata
  scripts/public-tier.py summary          markdown of what CI does not run
  scripts/public-tier.py require-private  are the private payloads imported? (local)
  scripts/public-tier.py record --checkout DIR
                                          regenerate the exclusions by running
                                          every target in a clean checkout DIR
"""
import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXCLUSIONS = ROOT / "scripts/private-fixture-tests.json"

# Each family of deliberately ignored payloads, a file that proves it was
# imported, and how to import it.
PAYLOADS = [
    ("fixtures/lifecycle/sta", "fixtures/lifecycle/sta/scenarios/spacex-transition.json",
     "python3 scripts/import-lifecycle-fixtures.py --sta <STA archive>"),
    ("fixtures/lifecycle/main", "fixtures/lifecycle/main/scenarios/spacex-transition.json",
     "python3 scripts/project-lifecycle-reference.py"),
    ("fixtures/lifecycle/main-t7", "fixtures/lifecycle/main-t7/scenarios/spacex-transition.json",
     "python3 scripts/project-lifecycle-reference.py"),
    ("fixtures/current/sta", "fixtures/current/sta/reports/milestone4-validation/live-market.capture.json",
     "python3 scripts/import-current-fixtures.py"),
    ("fixtures/path", "fixtures/path/sta/assets/prestocks-spacex.json",
     "python3 scripts/import-path-fixtures.py"),
    ("fixtures/migration", "fixtures/migration/pinned-programs/live-market.capture.json",
     "python3 scripts/import-migration-fixtures.py --sta <STA archive>"),
    ("fixtures/catalogue", None, "python3 scripts/import-catalogue-fixtures.py"),
]


def run(cmd, cwd=ROOT, check=True, capture=False):
    print("+ " + " ".join(cmd), flush=True)
    result = subprocess.run(cmd, cwd=cwd, text=True, capture_output=capture)
    if check and result.returncode != 0:
        if capture:
            sys.stdout.write(result.stdout)
            sys.stderr.write(result.stderr)
        sys.exit(result.returncode)
    return result


def metadata(cwd=ROOT):
    data = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"], cwd=cwd))
    packages = {}
    for package in data["packages"]:
        kinds = {kind for target in package["targets"] for kind in target["kind"]}
        tests = sorted(t["name"] for t in package["targets"] if "test" in t["kind"])
        packages[package["name"]] = {"kinds": kinds, "tests": tests}
    return packages


def load_exclusions(packages):
    data = json.loads(EXCLUSIONS.read_text())
    targets = data["targets"]
    for package, entries in targets.items():
        if package not in packages:
            sys.exit(f"{EXCLUSIONS.name}: unknown package {package}")
        for target, what in entries.items():
            if target != "lib" and target not in packages[package]["tests"]:
                sys.exit(f"{EXCLUSIONS.name}: {package} has no test target {target}; "
                         "remove the stale exclusion")
            if what != "all" and not (isinstance(what, list) and what):
                sys.exit(f"{EXCLUSIONS.name}: {package}/{target} must be \"all\" or a test list")
    return data


def selection(package, info, excluded):
    """(cargo target flags, per-target skip lists) for one package."""
    flags, skips = [], {}
    if "lib" in info["kinds"] and excluded.get("lib") != "all":
        flags.append("--lib")
        skips["lib"] = excluded.get("lib", [])
    if "bin" in info["kinds"]:
        flags.append("--bins")
    if "example" in info["kinds"]:
        flags.append("--examples")
    for name in info["tests"]:
        if excluded.get(name) == "all":
            continue
        flags += ["--test", name]
        skips[name] = excluded.get(name, [])
    return flags, skips


def listed_tests(package, target):
    selector = ["--lib"] if target == "lib" else ["--test", target]
    out = run(["cargo", "test", "--locked", "-q", "-p", package, *selector, "--", "--list",
               "--format", "terse"], capture=True).stdout
    return {line[: -len(": test")] for line in out.splitlines() if line.endswith(": test")}


def command_test(_args):
    packages = metadata()
    data = load_exclusions(packages)
    plans = {name: selection(name, info, data["targets"].get(name, {}))
             for name, info in packages.items()}
    # Build everything first, in parallel, so failures are compile failures.
    for package, (flags, _) in plans.items():
        run(["cargo", "test", "--locked", "--no-run", "-p", package, *flags])
    failed = []
    for package, (flags, skips) in plans.items():
        if "--bins" in flags or "--examples" in flags:
            extra = [f for f in ("--bins", "--examples") if f in flags]
            if run(["cargo", "test", "--locked", "-p", package, *extra], check=False).returncode:
                failed.append(f"{package} {' '.join(extra)}")
        for target, skip in skips.items():
            if skip:
                missing = sorted(set(skip) - listed_tests(package, target))
                if missing:
                    sys.exit(f"{EXCLUSIONS.name}: {package}/{target} no longer has "
                             f"{', '.join(missing)}; remove the stale exclusion")
            selector = ["--lib"] if target == "lib" else ["--test", target]
            filters = [arg for name in skip for arg in ("--skip", name)]
            cmd = ["cargo", "test", "--locked", "-p", package, *selector, "--"]
            cmd += (filters + ["--exact"]) if filters else []
            if run(cmd, check=False).returncode:
                failed.append(f"{package} {target}")
    print()
    print(public_summary(packages, data))
    if failed:
        sys.exit("public tier failed in: " + ", ".join(failed) + "\n"
                 "A test that needs a private payload belongs in "
                 f"{EXCLUSIONS.name} (scripts/public-tier.py record); anything else is a regression.")


def command_clippy(_args):
    packages = metadata()
    data = load_exclusions(packages)
    for package, info in packages.items():
        flags, _ = selection(package, info, data["targets"].get(package, {}))
        run(["cargo", "clippy", "--locked", "-p", package, *flags, "--", "-D", "warnings"])


def public_summary(packages, data):
    whole, tests = [], 0
    for package, entries in data["targets"].items():
        for target, what in entries.items():
            if what == "all":
                whole.append(f"{package}/{target}")
            else:
                tests += len(what)
    lines = [
        "### Public tier: what this run does not cover",
        "",
        "These suites read fixture payloads that are kept out of git on purpose "
        "(historical captures with provider-origin fields). They run in "
        "`make regression` on a machine with the payloads imported.",
        "",
        f"- **{len(whole)} test targets** that need the payloads to compile: "
        + ", ".join(f"`{w}`" for w in whole),
        f"- **{tests} tests** in other targets (listed in `scripts/{EXCLUSIONS.name}`)",
    ]
    for suite in data.get("browser", []):
        lines.append(f"- browser suite `{suite}`")
    return "\n".join(lines)


def command_check(_args):
    load_exclusions(metadata())
    print(f"{EXCLUSIONS.name}: every excluded target exists")


def command_summary(_args):
    text = public_summary(metadata(), json.loads(EXCLUSIONS.read_text()))
    target = os.environ.get("GITHUB_STEP_SUMMARY")
    if target:
        with open(target, "a", encoding="utf-8") as handle:
            handle.write(text + "\n")
    print(text)


def command_require_private(_args):
    missing = []
    for family, sentinel, how in PAYLOADS:
        if sentinel is None:
            version = json.loads((ROOT / family / "provenance.json").read_text())["current_version"]
            sentinel = f"{family}/{version}.json"
        if not (ROOT / sentinel).is_file():
            missing.append(f"  {family}: missing {sentinel}\n    import: {how}")
    if missing:
        sys.exit("private fixture payloads are not imported:\n" + "\n".join(missing)
                 + "\nThe public tier (make test-public) runs without them.")
    print("private fixture payloads present")


FAILURES = re.compile(r"^failures:\n((?:    \S+\n)+)", re.M)


def command_record(args):
    checkout = Path(args.checkout).resolve()
    for family, sentinel, _ in PAYLOADS:
        if sentinel and (checkout / sentinel).exists():
            sys.exit(f"{checkout} has private payloads ({sentinel}); record needs a clean checkout")
    packages = metadata(checkout)
    targets = {}
    for package, info in packages.items():
        runs = (["lib"] if "lib" in info["kinds"] else []) + info["tests"]
        for target in runs:
            selector = ["--lib"] if target == "lib" else ["--test", target]
            result = subprocess.run(["cargo", "test", "--locked", "-p", package, *selector,
                                     "--no-fail-fast"], cwd=checkout, text=True,
                                    capture_output=True)
            text = result.stdout + result.stderr
            if result.returncode == 0:
                continue
            if "could not compile" in text:
                targets.setdefault(package, {})[target] = "all"
            else:
                names = sorted({name.strip() for block in FAILURES.findall(text)
                                for name in block.splitlines()})
                if not names:
                    sys.exit(f"{package}/{target} failed without test failures:\n{text[-3000:]}")
                targets.setdefault(package, {})[target] = names
            print(f"{package}/{target}: {targets[package][target] if targets[package][target] == 'all' else len(targets[package][target])}", flush=True)
    data = json.loads(EXCLUSIONS.read_text()) if EXCLUSIONS.exists() else {}
    data["targets"] = targets
    data.setdefault("browser", ["test:cloud"])
    EXCLUSIONS.write_text(json.dumps(data, indent=1, sort_keys=True) + "\n")
    print(f"wrote {EXCLUSIONS}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("test", "clippy", "check", "summary", "require-private"):
        sub.add_parser(name)
    record = sub.add_parser("record")
    record.add_argument("--checkout", required=True)
    args = parser.parse_args()
    {"test": command_test, "clippy": command_clippy, "check": command_check,
     "summary": command_summary,
     "require-private": command_require_private, "record": command_record}[args.command](args)


if __name__ == "__main__":
    main()
