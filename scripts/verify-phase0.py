#!/usr/bin/env python3
"""Create a Phase 0 evidence bundle and report Pass, Fail, or Blocked gates."""

import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys


REQUIRED_FIXTURES = {
    "jpeg-fixture": {".jpg", ".jpeg"},
    "heic-fixture": {".heic", ".heif"},
    "raf-fixture": {".raf"},
    "orf-fixture": {".orf"},
}
REQUIRED_BUNDLE_CHECKS = {
    "rust-toolchain",
    "format",
    "clippy",
    "tests",
    "release-binaries",
    "decoder-probes",
    "original-integrity",
    "dependency-license-metadata",
    "dependency-phase0-plan",
    *REQUIRED_FIXTURES,
}


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def exit_code(results):
    statuses = {row[1] for row in results}
    if "Fail" in statuses:
        return 1
    if "Blocked" in statuses:
        return 2
    return 0


def add(results, check, status, detail, evidence=""):
    results.append((check, status, detail.replace("\t", " ").replace("\n", " "), evidence))


def run_check(repo, evidence, results, check, command):
    log = evidence / f"{check}.log"
    with log.open("wb") as output:
        completed = subprocess.run(command, cwd=repo, stdout=output, stderr=subprocess.STDOUT)
    status = "Pass" if completed.returncode == 0 else "Fail"
    add(results, check, status, f"exit {completed.returncode}", log.name)
    return completed.returncode == 0


def fixture_groups(paths):
    groups = {name: [] for name in REQUIRED_FIXTURES}
    for path in paths:
        suffix = path.suffix.lower()
        for name, suffixes in REQUIRED_FIXTURES.items():
            if suffix in suffixes:
                groups[name].append(path)
    return groups


def source_identity(repo):
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    diff = subprocess.check_output(["git", "diff", "--binary", "HEAD"], cwd=repo)
    untracked = subprocess.check_output(
        ["git", "ls-files", "--others", "--exclude-standard", "-z"], cwd=repo
    ).split(b"\0")
    state = hashlib.sha256(diff)
    for relative_bytes in sorted(path for path in untracked if path):
        relative = Path(os.fsdecode(relative_bytes))
        candidate = repo / relative
        if candidate.is_file():
            state.update(relative_bytes)
            state.update(b"\0")
            state.update(bytes.fromhex(digest(candidate)))
    return {"commit": commit, "dirty_diff_sha256": state.hexdigest()}


def load_bundle(path):
    with (path / "results.tsv").open(newline="") as source:
        reader = csv.DictReader(source, delimiter="\t")
        required_columns = {"check", "status", "detail", "evidence"}
        if not required_columns.issubset(reader.fieldnames or []):
            raise ValueError("results.tsv is missing required columns")
        results = list(reader)
    if any(row["status"] not in {"Pass", "Fail", "Blocked"} for row in results):
        raise ValueError("results.tsv contains an invalid status")
    return {
        "path": path,
        "source": json.loads((path / "source.json").read_text()),
        "host": json.loads((path / "host.json").read_text()),
        "results": results,
    }


def performance_receipt_error(value):
    if not isinstance(value, dict):
        return "top level must be an object"
    integers = {
        "indexed_photos": 10_000,
        "frame_count": 100,
        "distinct_grid_positions": 20,
        "grid_items_traversed": 500,
        "budget_us": 1,
        "p95_us": 0,
    }
    for name, minimum in integers.items():
        current = value.get(name)
        if isinstance(current, bool) or not isinstance(current, int) or current < minimum:
            return f"{name} must be an integer of at least {minimum}"
    if value["p95_us"] > value["budget_us"]:
        return "p95_us exceeds budget_us"
    if value.get("schema") != 1:
        return "schema must be 1"
    return None


def release_assessment(output, evidence_paths):
    results = []
    bundles = []
    for path in evidence_paths:
        try:
            bundles.append(load_bundle(path.resolve(strict=True)))
        except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
            add(results, "evidence-bundle", "Fail", f"cannot read {path}: {error}")
    identities = {json.dumps(bundle["source"], sort_keys=True) for bundle in bundles}
    add(
        results,
        "source-identity",
        "Pass" if bundles and len(identities) == 1 else "Fail",
        f"{len(bundles)} bundles, {len(identities)} source identities",
    )
    for bundle in bundles:
        statuses = {row["check"]: row["status"] for row in bundle["results"]}
        for check in sorted(REQUIRED_BUNDLE_CHECKS):
            status = statuses.get(check)
            add(
                results,
                f"{bundle['host'].get('system', 'unknown').lower()}-{check}",
                "Pass" if status == "Pass" else "Fail",
                f"bundle check is {status or 'missing'} in {bundle['path']}",
            )
    for system in ["Darwin", "Windows", "Linux"]:
        matches = [bundle for bundle in bundles if bundle["host"].get("system") == system]
        add(results, f"{system.lower()}-native", "Pass" if matches else "Blocked", f"{len(matches)} native bundles")
    linux_sessions = {
        bundle["host"].get("linux_session", "").lower()
        for bundle in bundles
        if bundle["host"].get("system") == "Linux"
    }
    for session in ["wayland", "x11"]:
        add(results, f"linux-{session}", "Pass" if session in linux_sessions else "Blocked", f"native {session} evidence")
    receipts = []
    for bundle in bundles:
        receipt = bundle["path"] / "gui-receipt.json"
        if receipt.exists():
            try:
                value = json.loads(receipt.read_text())
                if not isinstance(value, dict):
                    add(results, "gui-receipt-schema", "Fail", f"top level is not an object in {receipt}")
                elif value.get("source") != bundle["source"]:
                    add(results, "gui-receipt-identity", "Fail", f"source mismatch in {receipt}")
                else:
                    receipts.append((bundle, value))
            except (OSError, ValueError, json.JSONDecodeError) as error:
                add(results, "gui-receipt", "Fail", f"cannot read {receipt}: {error}")
    for system in ["Darwin", "Windows", "Linux"]:
        matches = [value for bundle, value in receipts if bundle["host"].get("system") == system]
        add(results, f"{system.lower()}-gui", "Pass" if matches else "Blocked", f"{len(matches)} matching GUI receipts")
    performance_receipts = []
    for bundle in bundles:
        receipt = bundle["path"] / "performance-receipt.json"
        if receipt.exists():
            try:
                value = json.loads(receipt.read_text())
                if error := performance_receipt_error(value):
                    add(results, "performance-receipt-schema", "Fail", f"{error} in {receipt}")
                elif value.get("source") != bundle["source"]:
                    add(results, "performance-receipt-identity", "Fail", f"source mismatch in {receipt}")
                else:
                    performance_receipts.append((bundle, value))
            except (OSError, ValueError, json.JSONDecodeError) as error:
                add(results, "performance-receipt", "Fail", f"cannot read {receipt}: {error}")
    for system in ["Darwin", "Windows", "Linux"]:
        matches = [
            value
            for bundle, value in performance_receipts
            if bundle["host"].get("system") == system
        ]
        add(
            results,
            f"{system.lower()}-performance",
            "Pass" if matches else "Blocked",
            f"{len(matches)} matching 10000-photo performance receipts",
        )
        add(
            results,
            f"{system.lower()}-interaction-memory-performance",
            "Blocked",
            "no source-bound producer exists yet for next-photo, exposure, and bounded-memory evidence",
        )
    for technology, system in [("voiceover", "Darwin"), ("nvda", "Windows"), ("orca", "Linux")]:
        passed = any(
            bundle["host"].get("system") == system
            and technology in value.get("verified_capabilities", [])
            for bundle, value in receipts
        )
        add(
            results,
            technology,
            "Pass" if passed else "Blocked",
            f"native {system} receipt" if passed else f"no native {system} receipt",
        )
    for system in ["Darwin", "Windows", "Linux"]:
        passed = any(
            bundle["host"].get("system") == system
            and "display-profile" in value.get("verified_capabilities", [])
            for bundle, value in receipts
        )
        add(
            results,
            f"{system.lower()}-display-profile",
            "Pass" if passed else "Blocked",
            f"native {system} display receipt" if passed else f"no native {system} display receipt",
        )
    if bundles:
        (output / "source.json").write_text(json.dumps(bundles[0]["source"], indent=2) + "\n")
    write_results(output, "release", results)
    return exit_code(results)


def write_license_report(repo, evidence, results):
    completed = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=repo,
        capture_output=True,
        text=True,
    )
    (evidence / "cargo-metadata.stderr.log").write_text(completed.stderr)
    if completed.returncode != 0:
        add(results, "dependency-license-metadata", "Fail", f"cargo metadata exit {completed.returncode}", "cargo-metadata.stderr.log")
        return
    metadata = json.loads(completed.stdout)
    packages = sorted(metadata["packages"], key=lambda package: (package["name"], package["version"]))
    with (evidence / "licenses.tsv").open("w", newline="") as target:
        writer = csv.writer(target, delimiter="\t", lineterminator="\n")
        writer.writerow(["name", "version", "source", "license", "license_file"])
        for package in packages:
            writer.writerow([
                package["name"],
                package["version"],
                package.get("source") or "workspace",
                package.get("license") or "",
                package.get("license_file") or "",
            ])
    missing = [package["name"] for package in packages if not package.get("license") and not package.get("license_file")]
    if missing:
        add(results, "dependency-license-metadata", "Fail", f"missing license metadata: {', '.join(missing)}", "licenses.tsv")
    else:
        add(results, "dependency-license-metadata", "Pass", f"{len(packages)} packages report a license or license file", "licenses.tsv")
    add(results, "dependency-phase0-plan", "Pass", "confirmed dependencies are pinned and release packaging remains a separate gate", "licenses.tsv")


def verify_toolchain(repo, results):
    manifest = (repo / "Cargo.toml").read_text()
    declared = re.search(r'^rust-version = "([0-9.]+)"$', manifest, re.MULTILINE)
    completed = subprocess.run(["rustc", "--version"], capture_output=True, text=True)
    detail = completed.stdout.strip() or completed.stderr.strip()
    if completed.returncode != 0 or not declared:
        add(results, "rust-toolchain", "Fail", detail or "workspace rust-version is missing")
        return
    installed = re.search(r"rustc ([0-9]+)\.([0-9]+)", detail)
    required = tuple(map(int, declared.group(1).split(".")[:2]))
    current = tuple(map(int, installed.groups())) if installed else (0, 0)
    add(results, "rust-toolchain", "Pass" if current >= required else "Fail", f"declared {declared.group(1)}; {detail}")


def write_results(evidence, mode, results):
    with (evidence / "results.tsv").open("w", newline="") as target:
        writer = csv.writer(target, delimiter="\t", lineterminator="\n")
        writer.writerow(["check", "status", "detail", "evidence"])
        writer.writerows(results)
    summary = {
        "schema": 1,
        "mode": mode,
        "host": platform.platform(),
        "machine": platform.machine(),
        "exit": exit_code(results),
        "counts": {status: sum(row[1] == status for row in results) for status in ["Pass", "Fail", "Blocked"]},
    }
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["mac-local", "release"])
    parser.add_argument("output", type=Path)
    parser.add_argument("inputs", nargs="*", type=Path)
    args = parser.parse_args(argv)
    repo = Path(os.environ.get("CREMA_REPO_ROOT", Path(__file__).resolve().parent.parent)).resolve()
    output = args.output.resolve()
    if output == repo or repo in output.parents:
        parser.error("evidence directory must stay outside the repository")
    try:
        output.mkdir(parents=True, exist_ok=False)
    except OSError as error:
        parser.error(f"cannot create evidence directory: {error}")

    if args.mode == "release":
        if not args.inputs:
            add_results = []
            add(add_results, "evidence-bundle", "Fail", "release assessment needs at least one evidence directory")
            write_results(output, "release", add_results)
            return 1
        code = release_assessment(output, args.inputs)
        print(output / "results.tsv")
        return code

    fixtures = [path.resolve(strict=True) for path in args.inputs]
    if any(output == path.parent or path.parent in output.parents for path in fixtures):
        parser.error("evidence directory must stay outside fixture folders")

    results = []
    identity = source_identity(repo)
    (output / "source.json").write_text(json.dumps(identity, indent=2) + "\n")
    (output / "host.json").write_text(json.dumps({
        "schema": 1,
        "system": platform.system(),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "linux_session": os.environ.get("XDG_SESSION_TYPE", ""),
    }, indent=2) + "\n")
    verify_toolchain(repo, results)
    host = platform.system()
    add(results, "macos-native-host", "Pass" if host == "Darwin" else "Blocked", f"host is {host}")
    for check, command in [
        ("format", ["cargo", "fmt", "--all", "--check"]),
        ("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
        ("tests", ["cargo", "test", "--workspace"]),
        ("release-binaries", ["cargo", "build", "--release", "-p", "crema-app", "--bins"]),
    ]:
        run_check(repo, output, results, check, command)

    before = {str(path): digest(path) for path in fixtures}
    (output / "fixture-hashes-before.json").write_text(json.dumps(before, indent=2) + "\n")
    groups = fixture_groups(fixtures)
    for name, members in groups.items():
        add(results, name, "Pass" if members else "Blocked", f"{len(members)} supplied", "fixture-hashes-before.json")
    if fixtures:
        run_check(
            repo,
            output,
            results,
            "decoder-probes",
            [sys.executable, str(repo / "scripts" / "verify-decode.py"), str(output / "decoder"), *map(str, fixtures)],
        )
    else:
        add(results, "decoder-probes", "Blocked", "no private fixtures supplied")
    after = {str(path): digest(path) for path in fixtures}
    (output / "fixture-hashes-after.json").write_text(json.dumps(after, indent=2) + "\n")
    add(results, "original-integrity", "Pass" if before == after else "Fail", f"{len(fixtures)} fixture hashes unchanged" if before == after else "one or more fixture hashes changed", "fixture-hashes-after.json")

    write_license_report(repo, output, results)
    add(results, "macos-display-color", "Blocked", "no instrumented comparison against the active macOS display profile")
    add(results, "voiceover", "Blocked", "no machine-recorded VoiceOver navigation evidence")
    write_results(output, "mac-local", results)
    code = exit_code(results)
    print(f"Phase 0 mac-local: Pass={sum(row[1] == 'Pass' for row in results)} Fail={sum(row[1] == 'Fail' for row in results)} Blocked={sum(row[1] == 'Blocked' for row in results)}")
    print(output / "results.tsv")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
