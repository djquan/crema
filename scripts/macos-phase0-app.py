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


MARKER = "crema-phase0-macos-session-v2\n"
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


def artifact_reference(evidence, path):
    resolved = path.resolve(strict=True)
    root = evidence.resolve(strict=True)
    if root not in resolved.parents:
        raise ValueError(f"artifact is outside the evidence bundle: {resolved}")
    return {"path": str(resolved.relative_to(root)), "sha256": digest(resolved)}


def copy_artifact(evidence, path, group):
    target_root = evidence / "macos-app" / "artifacts" / group
    target_root.mkdir(parents=True, exist_ok=True)
    source_hash = digest(path)
    target = target_root / f"{source_hash[:12]}-{path.name}"
    if target.exists() and digest(target) != source_hash:
        raise ValueError(f"artifact collision at {target}")
    if not target.exists():
        shutil.copy2(path, target)
    return artifact_reference(evidence, target)


def read_metric_session(path):
    with path.open(newline="") as source:
        events = list(csv.DictReader(source, delimiter="\t"))
    if not events or "session_id" not in events[0]:
        raise ValueError(f"metrics lack a session identity: {path}")
    session_ids = {event["session_id"] for event in events}
    if len(session_ids) != 1 or not next(iter(session_ids)):
        raise ValueError(f"metrics mix session identities: {path}")
    if any(event["event"] == "metrics_dropped" and int(event["value"]) for event in events):
        raise ValueError(f"metric overflow invalidates {path}")
    return next(iter(session_ids)), events


def metric_sessions(work):
    return [
        (path, *read_metric_session(path))
        for path in sorted((work / "metrics").glob("gui-metrics-*.tsv"))
    ]


def prepare(repo, evidence, photo_root):
    require_evidence(evidence)
    verified_source = json.loads((evidence / "source.json").read_text())
    if source_identity(repo) != verified_source:
        raise ValueError("worktree changed after the mac-local evidence bundle was created")
    work = evidence / "macos-app"
    work.mkdir(exist_ok=False)
    (work / ".crema-session").write_text(MARKER)
    (work / "metrics").mkdir()
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
            "CREMA_METRICS_DIR": str(work / "metrics"),
        },
    }
    with (app / "Contents" / "Info.plist").open("wb") as target:
        plistlib.dump(plist, target, sort_keys=True)
    run_logged(["codesign", "--force", "--sign", "-", str(app)], repo, work / "codesign.log")
    state = {
        "schema": 2,
        "photo_root": str(photo_root),
        "fixture_hashes": {str(path): digest(path) for path in fixture_paths},
        "source": verified_source,
        "host": json.loads((evidence / "host.json").read_text()),
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
    sessions = metric_sessions(work)
    if len(sessions) < 2:
        raise ValueError("GUI receipt needs separate save and restore launches")
    for path, _, events in sessions:
        names = {event["event"] for event in events}
        missing = {"gui_launch", "gui_frame_us", "gui_exit"} - names
        if missing:
            raise ValueError(f"{path} is missing: {', '.join(sorted(missing))}")

    pair = None
    for save_path, save_id, save_events in sessions:
        saved = {
            (event["interest"], event["value"])
            for event in save_events
            if event["event"] == "recipe_saved" and int(event["value"]) > 0
        }
        if not saved:
            continue
        for restore_path, restore_id, restore_events in sessions:
            if restore_path <= save_path:
                continue
            restored = {
                (event["interest"], event["value"])
                for event in restore_events
                if event["event"] == "recipe_restored" and int(event["value"]) > 0
            }
            if saved.intersection(restored):
                pair = (save_path, save_id, save_events, restore_path, restore_id, restore_events)
                break
        if pair:
            break
    if pair is None:
        raise ValueError("no second launch restored the first launch's saved asset and recipe")
    save_path, save_id, save_events, restore_path, restore_id, restore_events = pair

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
        "export": "export_finished",
    }
    if sum(event["event"] == "gui_selection" for event in save_events) < 2:
        raise ValueError("navigation observation needs at least two gui_selection metrics")
    for check, event_name in metric_requirements.items():
        if not any(
            event["event"] == event_name and int(event["value"]) > 0
            for event in save_events
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
    session_for_check = {
        "navigation": save_id,
        "exposure": save_id,
        "export": save_id,
        "save-reopen": restore_id,
        "close": restore_id,
    }
    for observation in observations:
        observation["session_id"] = session_for_check[observation["check"]]
    fixture_artifacts = [
        copy_artifact(evidence, Path(path), "fixtures")
        for path in sorted(state["fixture_hashes"])
    ]
    sidecar_artifacts = [copy_artifact(evidence, path, "sidecars") for path in sidecars]
    export_artifacts = [copy_artifact(evidence, path, "exports") for path in exports]
    receipt = {
        "schema": 2,
        "kind": "gui",
        "system": "Darwin",
        "source": state["source"],
        "host": state["host"],
        "app": artifact_reference(evidence, work / "Crema.app" / "Contents" / "MacOS" / "crema"),
        "fixtures": fixture_artifacts,
        "sidecars": sidecar_artifacts,
        "exports": export_artifacts,
        "sessions": [
            {
                "id": save_id,
                "role": "save",
                "normal_exit": True,
                "metrics": artifact_reference(evidence, save_path),
            },
            {
                "id": restore_id,
                "role": "restore",
                "normal_exit": True,
                "metrics": artifact_reference(evidence, restore_path),
            },
        ],
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
    candidates = []
    for metrics_path, session_id, events in metric_sessions(work):
        scan_counts = [
            int(event["value"]) for event in events if event["event"] == "scan_finished"
        ]
        if max(scan_counts, default=0) >= 10_000:
            candidates.append((metrics_path, session_id, events))
    if not candidates:
        raise ValueError("no metrics session indexed 10000 photos")
    metrics_path, session_id, events = candidates[-1]
    if not any(
        event["event"] == "gui_exit" and int(event["value"]) > 0 for event in events
    ):
        raise ValueError("performance run did not close normally")
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
        "schema": 2,
        "kind": "performance",
        "measurement": "gui-work",
        "cache_state": "warm",
        "system": "Darwin",
        "source": state["source"],
        "host": state["host"],
        "session_id": session_id,
        "app": artifact_reference(
            evidence, work / "Crema.app" / "Contents" / "MacOS" / "crema"
        ),
        "metrics": artifact_reference(evidence, metrics_path),
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
