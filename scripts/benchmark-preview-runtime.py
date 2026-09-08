#!/usr/bin/env python3
"""Measure the real preview runtime. Cache-cold does not mean OS disk-cold."""

import argparse
import csv
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import sys
import time


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def read_events(path):
    with path.open(newline="") as source:
        return list(csv.DictReader(source, delimiter="\t"))


def measurements(events, scenario=None):
    values = {}
    operation = None
    operation_index = -1
    cancellation = {}
    for event in events:
        name = event["event"]
        micros = int(event["micros"])
        if name == "operation_selected":
            operation = micros
            operation_index += 1
        elif name in {"first_usable_received", "target_received"} and operation is not None:
            prefix = "initial_" if scenario == "next" and operation_index == 0 else ""
            values.setdefault(prefix + name + "_us", []).append(micros - operation)
        elif name == "cancel_requested":
            cancellation[event["attempt"]] = micros
        elif name == "worker_reaped" and event["attempt"] in cancellation:
            values.setdefault("cancel_to_reap_us", []).append(micros - cancellation[event["attempt"]])
    if scenario == "next" and not values.get("target_received_us"):
        raise ValueError("no completed next-photo transition observations")
    for name in ["cache_hit", "cache_miss", "source_read_bytes", "worker_spawned", "obsolete_drop", "priority_drop"]:
        matching = [event for event in events if event["event"] == name]
        values[name] = [sum(int(event["value"]) for event in matching) if name == "source_read_bytes" else len(matching)]
    return values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("files", nargs="+", type=Path)
    parser.add_argument("--binary", type=Path, default=Path("target/release/crema-runtime-bench"))
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--compare", type=Path)
    parser.add_argument("--with-cancellation", action="store_true")
    args = parser.parse_args()
    if args.samples < 1:
        parser.error("samples must be positive")
    if len(args.files) < 2:
        parser.error("the next-photo scenario requires at least two fixtures")
    repo = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    files = [path.resolve(strict=True) for path in args.files]
    if output == repo or repo in output.parents or any(output == path.parent or path.parent in output.parents for path in files):
        parser.error("benchmark output must stay outside the repository and source folders")
    output.mkdir(parents=True, exist_ok=False)
    before = {str(path): digest(path) for path in files}
    (output / "hashes-before.json").write_text(json.dumps(before, indent=2) + "\n")
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    host = {"measurement_schema": 2, "platform": platform.platform(), "machine": platform.machine(), "cpu_count": os.cpu_count(), "commit": commit,
            "binary": str(args.binary.resolve()), "binary_sha256": digest(args.binary.resolve()), "fixture_hashes": before,
            "samples": args.samples, "os_cache": "unknown; integrity hashes read originals before measurements",
            "boundary": "real headless PreviewRuntime; excludes egui, texture upload, and presentation",
            "rss_scope": "macOS time -l maximum resident set size, not a simultaneous parent-plus-child sum"}
    (output / "host.json").write_text(json.dumps(host, indent=2) + "\n")
    all_values = {}
    runs = []
    try:
        for sample in range(args.samples):
            cache = output / f"cache-{sample}"
            scenarios = [("thumbnail", "app-cache-cold"), ("thumbnail", "persistent-warm"), ("viewer", "persistent-warm"), ("next", "mixed-next"), ("pressure", "result-pressure")]
            if args.with_cancellation:
                scenarios.extend([("cancel", "cache-disabled"), ("aba", "cache-disabled")])
            for index, (scenario, state) in enumerate(scenarios):
                name = f"{sample}-{index}-{scenario}-{state}"
                events_path = output / f"{name}.tsv"
                command = [str(args.binary.resolve()), scenario, "-" if state == "cache-disabled" else str(cache), str(events_path), *map(str, files)]
                if sys.platform == "darwin":
                    command = ["/usr/bin/time", "-l", *command]
                started = time.monotonic_ns()
                with (output / f"{name}.stdout").open("w") as stdout, (output / f"{name}.stderr").open("w") as stderr:
                    completed = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=300, check=False)
                if completed.returncode != 0:
                    raise RuntimeError(f"{name} failed; see its stderr and event log")
                elapsed = (time.monotonic_ns() - started) // 1000
                events = read_events(events_path)
                if any(event["event"] == "metrics_dropped" and int(event["value"]) for event in events):
                    raise RuntimeError("event buffer overflow invalidates this sample")
                values = measurements(events, scenario)
                if state == "persistent-warm" and scenario == "thumbnail":
                    if values["cache_hit"] != [1] or values["source_read_bytes"] != [0] or values["worker_spawned"] != [0]:
                        raise RuntimeError("warm thumbnail was not a zero-source-read cache hit")
                stderr = (output / f"{name}.stderr").read_text()
                rss = re.search(r"^\s*(\d+)\s+maximum resident set size\s*$", stderr, re.MULTILINE)
                if sys.platform == "darwin" and not rss:
                    raise RuntimeError("macOS time -l did not report maximum RSS")
                values["process_wall_us"] = [elapsed]
                if rss:
                    values["time_l_max_rss_bytes"] = [int(rss.group(1))]
                for metric, data in values.items():
                    all_values.setdefault((scenario, state, metric), []).extend(data)
                runs.append({"sample": sample, "scenario": scenario, "cache_state": state, "events": str(events_path), "values": values})
    finally:
        after = {str(path): digest(path) for path in files}
        (output / "hashes-after.json").write_text(json.dumps(after, indent=2) + "\n")
        if before != after:
            raise RuntimeError("original hashes changed")
    (output / "runs.json").write_text(json.dumps(runs, indent=2) + "\n")
    summary = []
    for (scenario, state, metric), values in sorted(all_values.items()):
        ordered = sorted(values)
        summary.append({"scenario": scenario, "cache_state": state, "metric": metric, "n": len(values), "p50": statistics.median(values), "p95": ordered[math.ceil(len(values) * .95) - 1]})
    with (output / "summary.tsv").open("w", newline="") as target:
        writer = csv.DictWriter(target, fieldnames=["scenario", "cache_state", "metric", "n", "p50", "p95"], delimiter="\t")
        writer.writeheader()
        writer.writerows(summary)
    if args.compare:
        previous = json.loads((args.compare / "host.json").read_text())
        if previous.get("measurement_schema") != host["measurement_schema"]:
            raise RuntimeError("comparison requires matching measurement schemas; older next-photo summaries included the initial open")
        if previous["fixture_hashes"] != before or previous["machine"] != host["machine"] or previous["platform"] != host["platform"]:
            raise RuntimeError("comparison requires the same fixture hashes and host platform")
        with (args.compare / "summary.tsv").open(newline="") as source:
            baseline = {(row["scenario"], row["cache_state"], row["metric"]): row for row in csv.DictReader(source, delimiter="\t")}
        changes = []
        for row in summary:
            old = baseline.get((row["scenario"], row["cache_state"], row["metric"]))
            if old:
                changes.append({**row, "baseline_p50": float(old["p50"]), "delta_p50": row["p50"] - float(old["p50"])})
        (output / "comparison.json").write_text(json.dumps(changes, indent=2) + "\n")
    print(f"{len(runs)} real runtime runs, unchanged originals, {output / 'summary.tsv'}")


if __name__ == "__main__":
    main()
