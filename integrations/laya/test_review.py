import json
import tempfile
import unittest
from pathlib import Path
from review import review_rows


class ReviewTests(unittest.TestCase):
    def test_recovery_does_not_create_a_correct_decision_label(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "shadow-test" / "events.jsonl"
            path.parent.mkdir()
            events = [
                {"kind": "incident_opened", "incident_id": "one", "request": {}, "evidence": {}},
                {"kind": "shadow_prediction", "incident_id": "one", "prediction": {"operation": "wait"}},
                {"kind": "execution_recovered", "incident_id": "one", "evidence": {"execution_succeeded": True}},
                {"kind": "session_ended", "dropped_records": 3},
            ]
            path.write_text("".join(json.dumps(e) + "\n" for e in events) + '{"interrupted":')
            row, = review_rows(Path(directory))
            self.assertIsNone(row["decision_correct"])
            self.assertIsNone(row["goal_achieved"])
            self.assertFalse(row["training_eligible"])
            self.assertTrue(row["incomplete_line"])
            self.assertEqual(row["dropped_records"], 3)


if __name__ == "__main__":
    unittest.main()
