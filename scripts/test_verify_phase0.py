import importlib.util
import csv
import json
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    "verify_phase0", Path(__file__).with_name("verify-phase0.py")
)
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)


class Phase0DomainTest(unittest.TestCase):
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
        self.assertEqual({name: len(paths) for name, paths in groups.items()}, {
            "jpeg-fixture": 1,
            "heic-fixture": 1,
            "raf-fixture": 1,
            "orf-fixture": 1,
        })

    def test_load_bundle_rejects_malformed_results(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.json").write_text("{}")
            (root / "host.json").write_text("{}")
            (root / "results.tsv").write_text("wrong\tcolumns\n")
            with self.assertRaisesRegex(ValueError, "missing required columns"):
                verify.load_bundle(root)

    def test_current_release_evidence_remains_blocked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = {"commit": "abc", "dirty_diff_sha256": "def"}
            systems = [
                ("Darwin", ""),
                ("Windows", ""),
                ("Linux", "wayland"),
                ("Linux", "x11"),
            ]
            bundles = []
            for index, (system, session) in enumerate(systems):
                bundle = root / f"bundle-{index}"
                bundle.mkdir()
                bundles.append(bundle)
                (bundle / "source.json").write_text(json.dumps(source))
                (bundle / "host.json").write_text(
                    json.dumps({"system": system, "linux_session": session})
                )
                with (bundle / "results.tsv").open("w", newline="") as target:
                    writer = csv.writer(target, delimiter="\t", lineterminator="\n")
                    writer.writerow(["check", "status", "detail", "evidence"])
                    for check in sorted(verify.REQUIRED_BUNDLE_CHECKS):
                        writer.writerow([check, "Pass", "verified", ""])
                capabilities = ["display-profile"]
                capabilities.extend(
                    {
                        "Darwin": ["voiceover"],
                        "Windows": ["nvda"],
                        "Linux": ["orca"],
                    }[system]
                )
                (bundle / "gui-receipt.json").write_text(
                    json.dumps(
                        {
                            "source": source,
                            "verified_capabilities": capabilities,
                        }
                    )
                )
                (bundle / "performance-receipt.json").write_text(
                    json.dumps(
                        {
                            "source": source,
                            "indexed_photos": 10_000,
                            "budget_us": 16_667,
                            "p95_us": 10_000,
                            "schema": 1,
                            "frame_count": 100,
                            "distinct_grid_positions": 20,
                            "grid_items_traversed": 500,
                        }
                    )
                )
            output = root / "release"
            output.mkdir()
            self.assertEqual(verify.release_assessment(output, bundles), 2)
            with (output / "results.tsv").open(newline="") as source_file:
                results = list(csv.DictReader(source_file, delimiter="\t"))
            blockers = {
                row["check"] for row in results if row["status"] == "Blocked"
            }
            self.assertIn("darwin-interaction-memory-performance", blockers)

    def test_release_rejects_malformed_performance_numbers(self):
        self.assertEqual(
            verify.performance_receipt_error(
                {
                    "schema": 1,
                    "indexed_photos": "10000",
                    "frame_count": 100,
                    "distinct_grid_positions": 20,
                    "grid_items_traversed": 500,
                    "budget_us": 16_667,
                    "p95_us": 400,
                }
            ),
            "indexed_photos must be an integer of at least 10000",
        )

    def test_performance_schema_requires_an_object(self):
        self.assertEqual(
            verify.performance_receipt_error([]), "top level must be an object"
        )


if __name__ == "__main__":
    unittest.main()
