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
    "python-tests",
    "release-binaries",
    "decoder-probes",
    "original-integrity",
    "dependency-license-metadata",
    "dependency-phase0-plan",
    *REQUIRED_FIXTURES,
}
FRAME_BUDGET_US = 16_667
GUI_WORKFLOW = {"navigation", "exposure", "save-reopen", "export", "close"}


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


def artifact_error(bundle, value, label):
    if not isinstance(value, dict) or set(value) != {"path", "sha256"}:
        return f"{label} must contain only path and sha256"
    relative = value.get("path")
    expected = value.get("sha256")
    if not isinstance(relative, str) or not relative or Path(relative).is_absolute():
        return f"{label} path must be bundle-relative"
    if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
        return f"{label} sha256 is invalid"
    try:
        root = bundle.resolve(strict=True)
        path = (bundle / relative).resolve(strict=True)
    except OSError as error:
        return f"{label} cannot be read: {error}"
    if root not in path.parents or not path.is_file():
        return f"{label} must reference a file inside the bundle"
    if digest(path) != expected:
        return f"{label} digest does not match"
    return None


def metric_events(bundle, reference, label):
    if error := artifact_error(bundle, reference, label):
        return error, []
    path = bundle / reference["path"]
    try:
        with path.open(newline="") as source:
            events = list(csv.DictReader(source, delimiter="\t"))
    except (OSError, csv.Error) as error:
        return f"{label} cannot be parsed: {error}", []
    required = {
        "session_id", "micros", "event", "asset", "purpose", "generation",
        "interest", "attempt", "value",
    }
    if not events or not required.issubset(events[0]):
        return f"{label} is empty or has an invalid header", []
    try:
        for row in events:
            for field in ["micros", "generation", "interest", "attempt", "value"]:
                if int(row[field]) < 0:
                    raise ValueError(field)
    except (KeyError, TypeError, ValueError):
        return f"{label} contains an invalid integer field", []
    return None, events


def successful_event(events, name):
    return any(row["event"] == name and int(row["value"]) > 0 for row in events)


def gui_receipt_error(value, bundle, source, host):
    if not isinstance(value, dict):
        return "top level must be an object"
    if value.get("schema") != 2 or value.get("kind") != "gui":
        return "schema must be 2 and kind must be gui"
    if value.get("source") != source or value.get("host") != host:
        return "source or host identity does not match the bundle"
    if value.get("system") != host.get("system"):
        return "system does not match the bundle host"
    if error := artifact_error(bundle, value.get("app"), "app"):
        return error
    for field in ["fixtures", "sidecars", "exports"]:
        artifacts = value.get(field)
        if not isinstance(artifacts, list) or not artifacts:
            return f"{field} must be a nonempty artifact list"
        for index, artifact in enumerate(artifacts):
            if error := artifact_error(bundle, artifact, f"{field}[{index}]"):
                return error

    sessions = value.get("sessions")
    if not isinstance(sessions, list) or len(sessions) != 2:
        return "sessions must contain one save launch and one restore launch"
    if {row.get("role") for row in sessions if isinstance(row, dict)} != {"save", "restore"}:
        return "sessions must have distinct save and restore roles"
    identifiers = [row.get("id") for row in sessions if isinstance(row, dict)]
    if len(identifiers) != 2 or any(not isinstance(item, str) or not item for item in identifiers):
        return "every session needs a nonempty id"
    if len(set(identifiers)) != 2:
        return "session ids must be unique"
    by_role = {}
    for session in sessions:
        if session.get("normal_exit") is not True:
            return f"session {session.get('id')} did not exit normally"
        error, events = metric_events(bundle, session.get("metrics"), f"session {session.get('id')} metrics")
        if error:
            return error
        if {row["session_id"] for row in events} != {session["id"]}:
            return f"session {session['id']} metrics contain another session id"
        if not successful_event(events, "gui_launch") or not successful_event(events, "gui_exit"):
            return f"session {session['id']} lacks a successful launch or exit"
        if any(row["event"] == "metrics_dropped" and int(row["value"]) for row in events):
            return f"session {session['id']} dropped metrics"
        by_role[session["role"]] = events
    save = by_role["save"]
    restore = by_role["restore"]
    if sum(row["event"] == "gui_selection" for row in save) < 2:
        return "save session lacks two navigation selections"
    for event in ["edit_render_received", "save_finished", "recipe_saved", "export_finished"]:
        if not successful_event(save, event):
            return f"save session lacks successful {event}"
    saved = {(row["interest"], row["value"]) for row in save if row["event"] == "recipe_saved"}
    restored = {
        (row["interest"], row["value"])
        for row in restore
        if row["event"] == "recipe_restored"
    }
    if not saved.intersection(restored):
        return "restore session does not reopen the saved asset and recipe"

    observations = value.get("ui_observations")
    if not isinstance(observations, list):
        return "ui_observations must be a list"
    checks = [row.get("check") for row in observations if isinstance(row, dict)]
    if set(checks) != GUI_WORKFLOW or len(checks) != len(GUI_WORKFLOW):
        return "ui_observations must contain each core workflow check exactly once"
    session_ids = set(identifiers)
    if any(row.get("status") != "Pass" or row.get("session_id") not in session_ids for row in observations):
        return "every UI observation must pass and name a receipt session"
    capabilities = value.get("verified_capabilities")
    if (
        not isinstance(capabilities, list)
        or len(capabilities) != len(GUI_WORKFLOW)
        or set(capabilities) != GUI_WORKFLOW
    ):
        return "verified_capabilities must contain only the core GUI workflow"
    return None


def percentile(values, fraction):
    index = min(len(values) - 1, int((len(values) - 1) * fraction))
    return values[index]


def performance_receipt_error(value, bundle=None, source=None, host=None):
    if not isinstance(value, dict):
        return "top level must be an object"
    if value.get("schema") != 2 or value.get("kind") != "performance":
        return "schema must be 2 and kind must be performance"
    if value.get("measurement") != "gui-work" or value.get("cache_state") != "warm":
        return "measurement must be gui-work with a warm cache"
    if value.get("budget_us") != FRAME_BUDGET_US:
        return f"budget_us must be the assessor-owned {FRAME_BUDGET_US}"
    if bundle is None:
        return "bundle is required to validate raw performance metrics"
    if value.get("source") != source or value.get("host") != host:
        return "source or host identity does not match the bundle"
    if value.get("system") != host.get("system"):
        return "system does not match the bundle host"
    if error := artifact_error(bundle, value.get("app"), "app"):
        return error
    error, events = metric_events(bundle, value.get("metrics"), "performance metrics")
    if error:
        return error
    session_id = value.get("session_id")
    if not isinstance(session_id, str) or {row["session_id"] for row in events} != {session_id}:
        return "performance metrics do not match session_id"
    if not successful_event(events, "gui_exit"):
        return "performance session did not exit normally"
    if any(row["event"] == "metrics_dropped" and int(row["value"]) for row in events):
        return "performance metrics were dropped"
    if any(row["event"] in {"cache_miss", "cache_store"} for row in events):
        return "performance metrics are not a warm-cache run"
    cache_hits = sum(row["event"] == "cache_hit" for row in events)
    indexed_photos = max(
        (int(row["value"]) for row in events if row["event"] == "scan_finished"),
        default=0,
    )
    frames = sorted(int(row["value"]) for row in events if row["event"] == "gui_frame_us")
    starts = {int(row["interest"]) for row in events if row["event"] == "grid_visible"}
    ends = [int(row["attempt"]) for row in events if row["event"] == "grid_visible"]
    traversed = max(ends, default=0) - min(starts, default=0)
    if indexed_photos < 10_000 or len(frames) < 100 or len(starts) < 20 or traversed < 500 or cache_hits == 0:
        return "raw metrics do not meet the 10000-photo warm traversal definition"
    computed = {
        "indexed_photos": indexed_photos,
        "cache_hits": cache_hits,
        "frame_count": len(frames),
        "distinct_grid_positions": len(starts),
        "grid_items_traversed": traversed,
        "p50_us": percentile(frames, 0.50),
        "p95_us": percentile(frames, 0.95),
        "p99_us": percentile(frames, 0.99),
        "max_us": frames[-1],
        "frames_over_budget": sum(frame > FRAME_BUDGET_US for frame in frames),
    }
    for name, expected in computed.items():
        current = value.get(name)
        if isinstance(current, bool) or not isinstance(current, int):
            return f"{name} must be an integer"
        if current != expected:
            return f"{name} does not match raw metrics"
    if computed["p95_us"] > FRAME_BUDGET_US:
        return "p95 GUI work exceeds the fixed frame budget"
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
            release_status = status if status in {"Pass", "Blocked"} else "Fail"
            add(
                results,
                f"{bundle['host'].get('system', 'unknown').lower()}-{check}",
                release_status,
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
                if error := gui_receipt_error(
                    value, bundle["path"], bundle["source"], bundle["host"]
                ):
                    add(results, "gui-receipt-schema", "Fail", f"{error} in {receipt}")
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
                if error := performance_receipt_error(
                    value, bundle["path"], bundle["source"], bundle["host"]
                ):
                    add(results, "performance-receipt-schema", "Fail", f"{error} in {receipt}")
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
    add(
        results,
        "dependency-phase0-plan",
        "Blocked",
        "no reviewed, source-bound dependency approval record was supplied",
        "licenses.tsv",
    )


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
        (
            "python-tests",
            [sys.executable, "-B", "-m", "unittest", "discover", "-s", "scripts", "-p", "test_*.py"],
        ),
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
