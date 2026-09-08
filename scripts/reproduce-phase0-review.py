#!/usr/bin/env python3
"""Reproduce the September 8 review findings without editing application files."""

import argparse
import csv
import importlib.util
import json
from pathlib import Path
import subprocess


def run(command, output, name, repo):
    with (output / f"{name}.log").open("x") as log:
        return subprocess.run(command, cwd=repo, stdout=log, stderr=subprocess.STDOUT).returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--heic-fixture", type=Path)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    if output == repo or repo in output.parents:
        parser.error("output must be outside the repository")
    if args.heic_fixture:
        fixture = args.heic_fixture.resolve(strict=True)
        if output == fixture.parent or fixture.parent in output.parents:
            parser.error("output must be outside the fixture folder")
    output.mkdir(parents=True, exist_ok=False)
    spec = importlib.util.spec_from_file_location("phase0_review_target", repo / "scripts/verify-phase0.py")
    verify = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(verify)
    source = {"commit": "synthetic-validator-input", "dirty_diff_sha256": "synthetic-validator-input"}
    bundle = output / "validator-input"
    bundle.mkdir()
    (bundle / "source.json").write_text(json.dumps(source))
    (bundle / "host.json").write_text(json.dumps({"system": "Darwin"}))
    with (bundle / "results.tsv").open("x") as target:
        writer = csv.writer(target, delimiter="\t")
        writer.writerow(["check", "status", "detail", "evidence"])
        for check in sorted(verify.REQUIRED_BUNDLE_CHECKS):
            writer.writerow([check, "Pass", "synthetic validator boundary input", ""])
    (bundle / "gui-receipt.json").write_text(json.dumps({"source": source}))
    performance = dict(source=source, schema=1, indexed_photos=10000, frame_count=100,
                       distinct_grid_positions=20, grid_items_traversed=500,
                       budget_us=1000000, p95_us=999999)
    (bundle / "performance-receipt.json").write_text(json.dumps(performance))
    assessment = output / "assessment"
    assessment.mkdir()
    verify.release_assessment(assessment, [bundle])
    with (assessment / "results.tsv").open() as target:
        checks = {row["check"]: row["status"] for row in csv.DictReader(target, delimiter="\t")}
    results = {
        "F5_source_only_gui_receipt_passes": checks.get("darwin-gui") == "Pass",
        "F6_one_second_frame_budget_passes": checks.get("darwin-performance") == "Pass",
    }
    dependencies = repo / "target/debug/deps"
    libraries = list(dependencies.glob("libquick_xml-*.rlib"))
    if libraries:
        library = max(libraries, key=lambda path: path.stat().st_mtime_ns)
        binary = output / "sidecar-probe"
        compiled = run([
            "rustc", "--edition=2024", "--crate-name", "phase0_sidecar_probe",
            str(repo / "docs/reviews/phase0-sidecar-probe.rs"),
            "--extern", f"quick_xml={library}", "-L", f"dependency={dependencies}",
            "-o", str(binary),
        ], output, "sidecar-compile", repo)
        if compiled:
            raise RuntimeError(f"sidecar probe compilation failed; read {output / 'sidecar-compile.log'}")
        results["F1_sidecar_recipe_crosses_originals"] = run(
            [str(binary), str(output / "sidecar-files")], output, "sidecar", repo
        ) == 0
    else:
        results["F1_sidecar_recipe_crosses_originals"] = None

    if args.heic_fixture:
        data = fixture.read_bytes()
        offset = data.index(b"nclx") + 4
        heic_root = output / "heic-variants"
        heic_root.mkdir()
        for name, primaries, transfer, matrix in [
            ("baseline", 1, 13, 6),
            ("unsupported-primaries", 11, 13, 6),
            ("unsupported-matrix", 1, 13, 8),
            ("undefined-color", 65535, 13, 65535),
        ]:
            variant = bytearray(data)
            variant[offset:offset + 6] = b"".join(
                value.to_bytes(2, "big") for value in (primaries, transfer, matrix)
            )
            (heic_root / f"{name}.heic").write_bytes(variant)
        results["F3_unsupported_heic_color_decodes"] = run([
            str(repo / "target/release/crema-probe"), "--fail-on-decode-error", str(heic_root),
        ], output, "heic", repo) == 0
    (output / "observed-defects.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results, indent=2))
    print("true means the reviewed defect reproduced; null means its prerequisite was absent")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
