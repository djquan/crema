import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "benchmark_preview_runtime",
    Path(__file__).with_name("benchmark-preview-runtime.py"),
)
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


class MeasurementsTest(unittest.TestCase):
    def test_next_requires_transition_observations(self):
        events = [{"event": name, "micros": str(micros), "attempt": "1", "value": "1"}
                  for name, micros in [("operation_selected", 0), ("first_usable_received", 10), ("target_received", 100)]]
        with self.assertRaisesRegex(ValueError, "next-photo transition"):
            benchmark.measurements(events, "next")

    def test_next_photo_excludes_initial_open(self):
        def event(name, micros):
            return {"event": name, "micros": str(micros), "attempt": "1", "value": "1"}
        events = [event("operation_selected", 0), event("first_usable_received", 100),
                  event("target_received", 1000), event("operation_selected", 2000),
                  event("first_usable_received", 2020), event("target_received", 2300),
                  event("operation_selected", 3000), event("first_usable_received", 3005),
                  event("target_received", 3100)]
        values = benchmark.measurements(events, "next")
        self.assertEqual(values["first_usable_received_us"], [20, 5])
        self.assertEqual(values["target_received_us"], [300, 100])
        self.assertEqual(values["initial_first_usable_received_us"], [100])
        self.assertEqual(values["initial_target_received_us"], [1000])
        self.assertEqual(
            benchmark.measurements(events[:3], "viewer")["first_usable_received_us"],
            [100],
        )


if __name__ == "__main__":
    unittest.main()
