#!/usr/bin/env python3
"""Probe originals without copying pixels, preserving hashes and per-run macOS RSS."""

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


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("files", nargs="+", type=Path)
    parser.add_argument("--probe", type=Path, default=Path("target/release/crema-probe"))
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    files = [path.resolve(strict=True) for path in args.files]
    if output == repo or repo in output.parents:
        parser.error("private-file reports must stay outside the repository")
    if any(output == path.parent or path.parent in output.parents for path in files):
        parser.error("reports must stay outside source folders")
    output.mkdir(parents=True, exist_ok=False)
    before = {str(path): digest(path) for path in files}
    (output / "hashes-before.json").write_text(json.dumps(before, indent=2) + "\n")
    (output / "host.json").write_text(json.dumps({
        "platform": platform.platform(), "machine": platform.machine(),
        "cpu_count": os.cpu_count(), "python": platform.python_version(),
        "probe": str(args.probe.resolve()),
        "dependencies": {"image": "0.25.10", "rawler": "0.8.0", "heif-oxide": "0.1.0", "eframe": "0.36.1"},
        "rss_scope": "one-file probe process and waited-for child maximum, macOS time -l; not a sum of concurrent RSS",
    }, indent=2) + "\n")
    rows = []
    try:
        for index, path in enumerate(files):
            command = [str(args.probe.resolve()), str(path)]
            if sys.platform == "darwin":
                command = ["/usr/bin/time", "-l", *command]
            with (output / f"{index}.tsv").open("w") as stdout, (output / f"{index}.stderr").open("w") as stderr:
                completed = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=90, check=False)
            with (output / f"{index}.tsv").open(newline="") as source:
                records = list(csv.DictReader(source, delimiter="\t"))
            if completed.returncode != 0 or len(records) != 1:
                raise RuntimeError(f"probe did not produce exactly one row for {path}")
            row = records[0]
            if row["outcome"] not in {"decoded", "unsupported", "failed"}:
                raise RuntimeError(f"invalid outcome for {path}")
            if row["outcome"] == "decoded":
                for field in ("source_width", "source_height", "preview_width", "preview_height"):
                    if int(row[field]) <= 0:
                        raise RuntimeError(f"invalid {field} for {path}")
            stderr = (output / f"{index}.stderr").read_text()
            match = re.search(r"^\s*(\d+)\s+maximum resident set size\s*$", stderr, re.MULTILINE)
            row["probe_max_rss_bytes"] = match.group(1) if match else "unknown:platform-counter-unavailable"
            if sys.platform == "darwin" and not match:
                raise RuntimeError(f"macOS time -l did not report maximum RSS for {path}")
            rows.append(row)
    finally:
        after = {str(path): digest(path) for path in files}
        (output / "hashes-after.json").write_text(json.dumps(after, indent=2) + "\n")
        if before != after:
            raise RuntimeError("original hashes changed")
    with (output / "capabilities.tsv").open("w", newline="") as target:
        writer = csv.DictWriter(target, fieldnames=list(rows[0]), delimiter="\t")
        writer.writeheader()
        writer.writerows(rows)
    print(f"{len(rows)} records, unchanged original hashes, report {output / 'capabilities.tsv'}")
    return 2 if any(row["outcome"] == "failed" for row in rows) else 0


if __name__ == "__main__":
    sys.exit(main())
