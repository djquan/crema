#!/usr/bin/env python3
"""Prepare and verify a real macOS Crema app bundle for Phase 0 UI checks."""

import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys


MARKER = "crema-phase0-macos-session-v1\n"
PHOTO_SUFFIXES = {".jpg", ".jpeg", ".heic", ".heif", ".raf", ".orf"}


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


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


def photos(root):
    return sorted(
        path for path in root.iterdir() if path.is_file() and path.suffix.lower() in PHOTO_SUFFIXES
    )


def fixture_hashes(state):
    return {path: digest(Path(path)) for path in state["fixture_hashes"]}


def require_evidence(evidence):
    if not (evidence / "source.json").is_file() or not (evidence / "results.tsv").is_file():
        raise ValueError("run verify-phase0.py mac-local first")


def run_logged(command, cwd, log):
    with log.open("wb") as output:
        completed = subprocess.run(command, cwd=cwd, stdout=output, stderr=subprocess.STDOUT)
    if completed.returncode:
        raise RuntimeError(f"command failed with exit {completed.returncode}; see {log}")


def prepare(repo, evidence, photo_root):
    require_evidence(evidence)
    verified_source = json.loads((evidence / "source.json").read_text())
    if source_identity(repo) != verified_source:
        raise ValueError("worktree changed after the mac-local evidence bundle was created")
    work = evidence / "macos-app"
    work.mkdir(exist_ok=False)
    (work / ".crema-session").write_text(MARKER)
    run_logged(
        ["cargo", "build", "--release", "-p", "crema-app", "--bin", "crema", "--bin", "crema-ui-fixture"],
        repo,
        work / "build.log",
    )
    if photo_root is None:
        photo_root = work / "photos"
        run_logged(
            [
                str(repo / "target" / "release" / "crema-ui-fixture"),
                "--count",
                "3",
                str(photo_root),
            ],
            repo,
            work / "fixture.log",
        )
    else:
        photo_root = photo_root.resolve(strict=True)
    fixture_paths = photos(photo_root)
    if not fixture_paths:
        raise ValueError(f"no supported photos in {photo_root}")

    app = work / "Crema.app"
    binary_dir = app / "Contents" / "MacOS"
    binary_dir.mkdir(parents=True)
    shutil.copy2(repo / "target" / "release" / "crema", binary_dir / "crema")
    plist = {
        "CFBundleExecutable": "crema",
        "CFBundleIdentifier": "com.danielquan.crema.phase0",
        "CFBundleName": "Crema",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": "0.1.0",
        "NSHighResolutionCapable": True,
        "LSEnvironment": {
            "CREMA_ROOT": str(photo_root),
            "CREMA_CACHE_ROOT": str(work / "cache"),
            "CREMA_METRICS": str(work / "gui-metrics.tsv"),
        },
    }
    with (app / "Contents" / "Info.plist").open("wb") as target:
        plistlib.dump(plist, target, sort_keys=True)
    run_logged(["codesign", "--force", "--sign", "-", str(app)], repo, work / "codesign.log")
    state = {
        "schema": 1,
        "photo_root": str(photo_root),
        "fixture_hashes": {str(path): digest(path) for path in fixture_paths},
        "source": verified_source,
    }
    (work / "session.json").write_text(json.dumps(state, indent=2) + "\n")
    doctor(evidence)


def doctor(evidence):
    work = evidence / "macos-app"
    state = json.loads((work / "session.json").read_text())
    binary = work / "Crema.app" / "Contents" / "MacOS" / "crema"
    kind = subprocess.check_output(["file", "-b", str(binary)], text=True).strip()
    if "Mach-O" not in kind:
        raise ValueError(f"app executable is not Mach-O: {kind}")
    subprocess.run(["codesign", "--verify", "--strict", str(work / "Crema.app")], check=True)
    current = fixture_hashes(state)
    if current != state["fixture_hashes"]:
        raise ValueError("photo fixtures changed after prepare")
    print(kind)
    print(work / "Crema.app")


def finish(evidence):
    require_evidence(evidence)
    work = evidence / "macos-app"
    state = json.loads((work / "session.json").read_text())
    metrics_path = work / "gui-metrics.tsv"
    with metrics_path.open(newline="") as source:
        events = list(csv.DictReader(source, delimiter="\t"))
    names = {event["event"] for event in events}
    required = {"gui_launch", "gui_frame_us", "gui_exit"}
    missing = required - names
    if missing:
        raise ValueError(f"GUI metrics are missing: {', '.join(sorted(missing))}")
    if any(event["event"] == "metrics_dropped" and int(event["value"]) for event in events):
        raise ValueError("GUI metric overflow invalidates the receipt")
    observations_root = work / "observations"
    observations = [
        json.loads(path.read_text()) for path in sorted(observations_root.glob("*.json"))
    ]
    required_observations = {"navigation", "exposure", "save-reopen", "export", "close"}
    passed_observations = {
        row["check"] for row in observations if row["status"] == "Pass"
    }
    missing_observations = required_observations - passed_observations
    if missing_observations:
        raise ValueError(
            f"UI observations are missing: {', '.join(sorted(missing_observations))}"
        )
    metric_requirements = {
        "exposure": "edit_render_received",
        "save-reopen": "save_finished",
        "export": "export_finished",
        "close": "gui_exit",
    }
    if sum(event["event"] == "gui_selection" for event in events) < 2:
        raise ValueError("navigation observation needs at least two gui_selection metrics")
    for check, event_name in metric_requirements.items():
        if not any(
            event["event"] == event_name and int(event["value"]) > 0 for event in events
        ):
            raise ValueError(f"{check} observation has no successful {event_name} metric")
    current = fixture_hashes(state)
    if current != state["fixture_hashes"]:
        raise ValueError("photo fixtures changed during the GUI session")
    photo_root = Path(state["photo_root"])
    sidecars = sorted(photo_root.glob("*.xmp"))
    exports = sorted(photo_root.glob("*-crema.jpg"))
    if not sidecars:
        raise ValueError("GUI session produced no Crema XMP sidecar")
    if not exports:
        raise ValueError("GUI session produced no JPEG export")
    for export in exports:
        if b"ICC_PROFILE\0" not in export.read_bytes():
            raise ValueError(f"JPEG export has no embedded ICC profile: {export}")
    receipt = {
        "schema": 1,
        "system": "Darwin",
        "source": state["source"],
        "app_sha256": digest(work / "Crema.app" / "Contents" / "MacOS" / "crema"),
        "fixture_hashes": current,
        "sidecars": {str(path): digest(path) for path in sidecars},
        "exports": {str(path): digest(path) for path in exports},
        "metric_events": sorted(names),
        "ui_observations": observations,
        "verified_capabilities": sorted(required_observations),
    }
    target = evidence / "gui-receipt.json"
    with target.open("x") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    print(target)


def percentile(values, fraction):
    index = min(len(values) - 1, int((len(values) - 1) * fraction))
    return values[index]


def performance(evidence):
    require_evidence(evidence)
    work = evidence / "macos-app"
    state = json.loads((work / "session.json").read_text())
    metrics_path = work / "gui-metrics.tsv"
    with metrics_path.open(newline="") as source:
        events = list(csv.DictReader(source, delimiter="\t"))
    if not any(event["event"] == "gui_exit" for event in events):
        raise ValueError("performance run did not close normally")
    if any(event["event"] == "metrics_dropped" and int(event["value"]) for event in events):
        raise ValueError("GUI metric overflow invalidates the performance receipt")
    if any(event["event"] in {"cache_miss", "cache_store"} for event in events):
        raise ValueError("performance receipt requires a warm-cache run")
    cache_hits = sum(event["event"] == "cache_hit" for event in events)
    if not cache_hits:
        raise ValueError("performance run recorded no cache hits")
    scan_counts = [
        int(event["value"]) for event in events if event["event"] == "scan_finished"
    ]
    indexed_photos = max(scan_counts, default=0)
    if indexed_photos < 10_000:
        raise ValueError(f"performance receipt needs 10000 indexed photos; found {indexed_photos}")
    frames = sorted(int(event["value"]) for event in events if event["event"] == "gui_frame_us")
    if len(frames) < 100:
        raise ValueError(f"performance run needs at least 100 GUI frames; found {len(frames)}")
    visible_starts = {
        int(event["interest"]) for event in events if event["event"] == "grid_visible"
    }
    visible_ends = [
        int(event["attempt"]) for event in events if event["event"] == "grid_visible"
    ]
    if len(visible_starts) < 20 or not visible_ends:
        raise ValueError("performance run needs at least 20 distinct visible grid positions")
    visible_span = max(visible_ends) - min(visible_starts)
    if visible_span < 500:
        raise ValueError(f"performance run traversed only {visible_span} grid items")
    budget_us = 16_667
    receipt = {
        "schema": 1,
        "system": "Darwin",
        "source": state["source"],
        "app_sha256": digest(work / "Crema.app" / "Contents" / "MacOS" / "crema"),
        "metrics_sha256": digest(metrics_path),
        "indexed_photos": indexed_photos,
        "cache_hits": cache_hits,
        "frame_count": len(frames),
        "distinct_grid_positions": len(visible_starts),
        "grid_items_traversed": visible_span,
        "budget_us": budget_us,
        "p50_us": percentile(frames, 0.50),
        "p95_us": percentile(frames, 0.95),
        "p99_us": percentile(frames, 0.99),
        "max_us": frames[-1],
        "frames_over_budget": sum(value > budget_us for value in frames),
        "limitation": "GUI work duration does not prove monitor presentation timing",
    }
    if receipt["p95_us"] > budget_us:
        raise ValueError(
            f"p95 GUI work {receipt['p95_us']} us exceeds {budget_us} us frame budget"
        )
    target = evidence / "performance-receipt.json"
    with target.open("x") as output:
        json.dump(receipt, output, indent=2)
        output.write("\n")
    print(target)


def observe(evidence, check, detail):
    if check is None:
        raise ValueError("observe requires navigation, exposure, save-reopen, export, or close")
    root = evidence / "macos-app" / "observations"
    root.mkdir(exist_ok=True)
    path = root / f"{check}.json"
    with path.open("x") as target:
        json.dump(
            {
                "check": check,
                "status": "Pass",
                "detail": detail or "observed in the real Crema app",
            },
            target,
            indent=2,
        )
        target.write("\n")
    print(path)


def cleanup(evidence):
    work = evidence / "macos-app"
    marker = work / ".crema-session"
    if marker.read_text() != MARKER:
        raise ValueError(f"refusing to remove unmarked path: {work}")
    app = work / "Crema.app"
    cache = work / "cache"
    if app.exists():
        shutil.rmtree(app)
    if cache.exists():
        shutil.rmtree(cache)
    print(f"removed the disposable app and cache under {work}; raw evidence remains")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command", choices=["prepare", "doctor", "observe", "finish", "performance", "cleanup"]
    )
    parser.add_argument("evidence", type=Path)
    parser.add_argument(
        "observation",
        nargs="?",
        choices=["navigation", "exposure", "save-reopen", "export", "close"],
    )
    parser.add_argument("--photos", type=Path)
    parser.add_argument("--detail")
    args = parser.parse_args(argv)
    if platform.system() != "Darwin":
        parser.error("this helper requires macOS")
    repo = Path(os.environ.get("CREMA_REPO_ROOT", Path(__file__).resolve().parent.parent)).resolve()
    evidence = args.evidence.resolve(strict=True)
    photo_root = args.photos or (Path(value) if (value := os.environ.get("CREMA_ROOT")) else None)
    try:
        if args.command == "prepare":
            prepare(repo, evidence, photo_root)
        elif args.command == "doctor":
            doctor(evidence)
        elif args.command == "observe":
            observe(evidence, args.observation, args.detail)
        elif args.command == "finish":
            finish(evidence)
        elif args.command == "performance":
            performance(evidence)
        else:
            cleanup(evidence)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
