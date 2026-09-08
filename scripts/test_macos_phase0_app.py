import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    "macos_phase0_app", Path(__file__).with_name("macos-phase0-app.py")
)
macos = importlib.util.module_from_spec(spec)
spec.loader.exec_module(macos)


class MacosEvidenceTest(unittest.TestCase):
    def make_evidence(self, root, indexed):
        (root / "source.json").write_text("{}")
        (root / "results.tsv").write_text("check\tstatus\tdetail\tevidence\n")
        work = root / "macos-app"
        binary = work / "Crema.app" / "Contents" / "MacOS" / "crema"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"binary")
        (work / "session.json").write_text(
            json.dumps({"source": {}, "fixture_hashes": {}})
        )
        rows = [
            "micros\tevent\tasset\tpurpose\tgeneration\tinterest\tattempt\tvalue\n"
            f"1\tscan_finished\t\t\t1\t0\t0\t{indexed}\n"
            "2\tcache_hit\t1\tThumbnail\t1\t1\t1\t1\n"
        ]
        rows.extend(
            f"{index + 3}\tgui_frame_us\t\t\t0\t0\t0\t400\n"
            for index in range(100)
        )
        rows.extend(
            f"{index + 103}\tgrid_visible\t\t\t0\t{index * 30}\t{index * 30 + 24}\t25\n"
            for index in range(20)
        )
        rows.extend(
            [
                "123\tgui_exit\t\t\t0\t0\t0\t1\n",
                "124\tmetrics_dropped\t\t\t0\t0\t0\t0\n",
            ]
        )
        (work / "gui-metrics.tsv").write_text("".join(rows))

    def test_performance_receipt_uses_scan_complete_count(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_evidence(root, 10_000)
            macos.performance(root)
            receipt = json.loads((root / "performance-receipt.json").read_text())
            self.assertEqual(receipt["indexed_photos"], 10_000)
            self.assertEqual(receipt["p95_us"], 400)

    def test_performance_receipt_rejects_partial_scan(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_evidence(root, 9_999)
            with self.assertRaisesRegex(ValueError, "10000 indexed photos"):
                macos.performance(root)

    def test_observation_is_create_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "macos-app").mkdir()
            macos.observe(root, "navigation", "moved")
            with self.assertRaises(FileExistsError):
                macos.observe(root, "navigation", "moved again")


if __name__ == "__main__":
    unittest.main()
