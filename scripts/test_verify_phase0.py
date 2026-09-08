import csv
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    "verify_phase0", Path(__file__).with_name("verify-phase0.py")
)
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)


HEADER = "session_id\tmicros\tevent\tasset\tpurpose\tgeneration\tinterest\tattempt\tvalue\n"


class Phase0DomainTest(unittest.TestCase):
    def artifact(self, bundle, name, contents=b"artifact"):
        path = bundle / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(contents)
        return {"path": name, "sha256": verify.digest(path)}

    def metrics(self, bundle, name, session_id, rows):
        body = [HEADER]
        for micros, event, asset, interest, attempt, value in rows:
            body.append(
                f"{session_id}\t{micros}\t{event}\t{asset}\tViewer\t1\t{interest}\t{attempt}\t{value}\n"
            )
        return self.artifact(bundle, name, "".join(body).encode())

    def make_bundle(self, root, system="Darwin"):
        bundle = root / f"bundle-{system.lower()}"
        bundle.mkdir()
        source = {"commit": "abc", "dirty_diff_sha256": "def"}
        host = {"system": system, "linux_session": ""}
        (bundle / "source.json").write_text(json.dumps(source))
        (bundle / "host.json").write_text(json.dumps(host))
        with (bundle / "results.tsv").open("w", newline="") as target:
            writer = csv.writer(target, delimiter="\t", lineterminator="\n")
            writer.writerow(["check", "status", "detail", "evidence"])
            for check in sorted(verify.REQUIRED_BUNDLE_CHECKS):
                writer.writerow([check, "Pass", "verified", ""])

        app = self.artifact(bundle, "app/crema")
        fixture = self.artifact(bundle, "artifacts/photo.jpg", b"original")
        sidecar = self.artifact(bundle, "artifacts/photo.xmp", b"owned xmp")
        export = self.artifact(bundle, "artifacts/photo-crema.jpg", b"jpeg")
        save_metrics = self.metrics(
            bundle,
            "metrics/save.tsv",
            "save-session",
            [
                (1, "gui_launch", "", 0, 0, 1),
                (2, "gui_selection", "asset", 0, 0, 1),
                (3, "gui_selection", "asset", 0, 0, 1),
                (4, "edit_render_received", "asset", 0, 0, 1),
                (5, "save_finished", "asset", 0, 0, 1),
                (6, "recipe_saved", "asset", 42, 0, 600),
                (7, "export_finished", "asset", 0, 0, 1),
                (8, "gui_exit", "", 0, 0, 1),
                (9, "metrics_dropped", "", 0, 0, 0),
            ],
        )
        restore_metrics = self.metrics(
            bundle,
            "metrics/restore.tsv",
            "restore-session",
            [
                (1, "gui_launch", "", 0, 0, 1),
                (2, "recipe_restored", "asset", 42, 0, 600),
                (3, "gui_exit", "", 0, 0, 1),
                (4, "metrics_dropped", "", 0, 0, 0),
            ],
        )
        observations = [
            {
                "check": check,
                "status": "Pass",
                "session_id": (
                    "restore-session" if check in {"save-reopen", "close"} else "save-session"
                ),
            }
            for check in sorted(verify.GUI_WORKFLOW)
        ]
        gui = {
            "schema": 2,
            "kind": "gui",
            "system": system,
            "source": source,
            "host": host,
            "app": app,
            "fixtures": [fixture],
            "sidecars": [sidecar],
            "exports": [export],
            "sessions": [
                {
                    "id": "save-session",
                    "role": "save",
                    "normal_exit": True,
                    "metrics": save_metrics,
                },
                {
                    "id": "restore-session",
                    "role": "restore",
                    "normal_exit": True,
                    "metrics": restore_metrics,
                },
            ],
            "ui_observations": observations,
            "verified_capabilities": sorted(verify.GUI_WORKFLOW),
        }
        (bundle / "gui-receipt.json").write_text(json.dumps(gui))

        performance_rows = [
            (1, "gui_launch", "", 0, 0, 1),
            (2, "scan_finished", "", 0, 0, 10_000),
            (3, "cache_hit", "asset", 0, 0, 1),
        ]
        performance_rows.extend(
            (10 + index, "gui_frame_us", "", 0, 0, 400) for index in range(100)
        )
        performance_rows.extend(
            (200 + index, "grid_visible", "", index * 30, index * 30 + 24, 25)
            for index in range(20)
        )
        performance_rows.extend(
            [(300, "gui_exit", "", 0, 0, 1), (301, "metrics_dropped", "", 0, 0, 0)]
        )
        raw_metrics = self.metrics(
            bundle, "metrics/performance.tsv", "performance-session", performance_rows
        )
        performance = {
            "schema": 2,
            "kind": "performance",
            "measurement": "gui-work",
            "cache_state": "warm",
            "system": system,
            "source": source,
            "host": host,
            "session_id": "performance-session",
            "app": app,
            "metrics": raw_metrics,
            "indexed_photos": 10_000,
            "cache_hits": 1,
            "frame_count": 100,
            "distinct_grid_positions": 20,
            "grid_items_traversed": 594,
            "budget_us": verify.FRAME_BUDGET_US,
            "p50_us": 400,
            "p95_us": 400,
            "p99_us": 400,
            "max_us": 400,
            "frames_over_budget": 0,
        }
        (bundle / "performance-receipt.json").write_text(json.dumps(performance))
        return bundle, source, host, gui

    def test_exit_codes_distinguish_failure_and_blockers(self):
        self.assertEqual(verify.exit_code([("one", "Pass", "", "")]), 0)
        self.assertEqual(verify.exit_code([("one", "Blocked", "", "")]), 2)
        self.assertEqual(
            verify.exit_code([("one", "Blocked", "", ""), ("two", "Fail", "", "")]),
            1,
        )

    def test_fixture_groups_are_explicit_and_case_insensitive(self):
        groups = verify.fixture_groups(
            [Path("a.JPEG"), Path("b.HEIC"), Path("c.RAF"), Path("d.ORF"), Path("ignored.png")]
        )
        self.assertEqual(
            {name: len(paths) for name, paths in groups.items()},
            {"jpeg-fixture": 1, "heic-fixture": 1, "raf-fixture": 1, "orf-fixture": 1},
        )

    def test_load_bundle_rejects_malformed_results(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.json").write_text("{}")
            (root / "host.json").write_text("{}")
            (root / "results.tsv").write_text("wrong\tcolumns\n")
            with self.assertRaisesRegex(ValueError, "missing required columns"):
                verify.load_bundle(root)

    def test_valid_native_receipts_pass_their_individual_gates(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle, _, _, _ = self.make_bundle(root)
            output = root / "release"
            output.mkdir()
            self.assertEqual(verify.release_assessment(output, [bundle]), 2)
            with (output / "results.tsv").open(newline="") as source:
                results = {
                    row["check"]: row["status"]
                    for row in csv.DictReader(source, delimiter="\t")
                }
            self.assertEqual(results["darwin-gui"], "Pass")
            self.assertEqual(results["darwin-performance"], "Pass")

    def test_release_preserves_a_blocked_native_check(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle, _, _, _ = self.make_bundle(root)
            with (bundle / "results.tsv").open(newline="") as source:
                rows = list(csv.DictReader(source, delimiter="\t"))
            for row in rows:
                if row["check"] == "raf-fixture":
                    row["status"] = "Blocked"
                    row["detail"] = "no representative RAF was supplied"
            with (bundle / "results.tsv").open("w", newline="") as target:
                writer = csv.DictWriter(
                    target,
                    fieldnames=["check", "status", "detail", "evidence"],
                    delimiter="\t",
                    lineterminator="\n",
                )
                writer.writeheader()
                writer.writerows(rows)

            output = root / "release"
            output.mkdir()
            self.assertEqual(verify.release_assessment(output, [bundle]), 2)
            with (output / "results.tsv").open(newline="") as source:
                results = {
                    row["check"]: row["status"]
                    for row in csv.DictReader(source, delimiter="\t")
                }
            self.assertEqual(results["darwin-raf-fixture"], "Blocked")

    def test_source_only_gui_receipt_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertIn(
                "schema must be 2",
                verify.gui_receipt_error({"source": {}}, root, {}, {"system": "Darwin"}),
            )

    def test_gui_receipt_rejects_duplicate_session_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle, source, host, gui = self.make_bundle(root)
            gui["sessions"][1]["id"] = "save-session"
            self.assertEqual(
                verify.gui_receipt_error(gui, bundle, source, host),
                "session ids must be unique",
            )

    def test_gui_receipt_rejects_tampered_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle, source, host, gui = self.make_bundle(root)
            (bundle / gui["exports"][0]["path"]).write_bytes(b"tampered")
            self.assertIn(
                "digest does not match",
                verify.gui_receipt_error(gui, bundle, source, host),
            )

    def test_core_gui_receipt_cannot_claim_assistive_or_display_proof(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle, source, host, gui = self.make_bundle(root)
            gui["verified_capabilities"].extend(["voiceover", "display-profile"])
            self.assertEqual(
                verify.gui_receipt_error(gui, bundle, source, host),
                "verified_capabilities must contain only the core GUI workflow",
            )

    def test_performance_budget_is_assessor_owned(self):
        self.assertEqual(
            verify.performance_receipt_error(
                {
                    "schema": 2,
                    "kind": "performance",
                    "measurement": "gui-work",
                    "cache_state": "warm",
                    "budget_us": 1_000_000,
                }
            ),
            f"budget_us must be the assessor-owned {verify.FRAME_BUDGET_US}",
        )

    def test_performance_aggregates_are_recomputed_from_raw_metrics(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle, source, host, _ = self.make_bundle(root)
            receipt = json.loads((bundle / "performance-receipt.json").read_text())
            receipt["p95_us"] = 399
            self.assertEqual(
                verify.performance_receipt_error(receipt, bundle, source, host),
                "p95_us does not match raw metrics",
            )

    def test_performance_schema_requires_an_object(self):
        self.assertEqual(verify.performance_receipt_error([]), "top level must be an object")


if __name__ == "__main__":
    unittest.main()
