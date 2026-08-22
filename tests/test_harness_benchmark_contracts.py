from __future__ import annotations

import json
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
FOCUS_TIMER = REPO_ROOT / "config" / "harness-benchmarks" / "focus-timer.json"
EXPECTED_PROMPT = (
    "Use write exactly once create index.html. tiny offline Focus Timer, circular 25-min "
    "countdown, Start/Pause Reset, completed-session localStorage. HARD LIMIT 90 nonblank "
    "lines, 1200 words. no dependencies. Do not call write_file. reply DONE."
)


class HarnessBenchmarkContractTests(unittest.TestCase):
    def test_focus_timer_contract_preserves_prompt_and_fail_closed_scoring(self) -> None:
        contract = json.loads(FOCUS_TIMER.read_text(encoding="utf-8"))

        self.assertEqual(contract["schema"], 1)
        self.assertEqual(contract["id"], "focus-timer-v1")
        self.assertEqual(contract["prompt"], EXPECTED_PROMPT)
        self.assertEqual(contract["workspace"]["allowed_created_files"], ["index.html"])
        self.assertEqual(contract["hard_gates"]["required_tool_calls"], {"write": 1})
        self.assertEqual(contract["hard_gates"]["forbidden_tool_calls"], ["write_file"])
        self.assertEqual(contract["hard_gates"]["final_response"], "DONE")
        self.assertEqual(contract["hard_gates"]["maximum_nonblank_lines"], 90)
        self.assertEqual(contract["hard_gates"]["maximum_words"], 1200)
        self.assertFalse(contract["hard_gates"]["external_dependencies"])
        self.assertFalse(contract["hard_gates"]["network_requests"])
        self.assertEqual(contract["measurement"]["artifact_encoding"], "utf-8")
        self.assertIn("Unicode-trimmed", contract["measurement"]["nonblank_lines"])
        self.assertIn("Unicode whitespace", contract["measurement"]["words"])
        self.assertIn("network denied", contract["measurement"]["network_check"])
        self.assertEqual(contract["scoring"]["hard_gate_failure_score"], 0)
        checks = {check["id"] for check in contract["functional_checks"]}
        self.assertEqual(
            checks,
            {
                "circular-display",
                "initial-duration",
                "start-pause",
                "reset",
                "completed-session-persistence",
            },
        )
        self.assertEqual(
            len(checks) * contract["scoring"]["points_per_functional_check"],
            contract["scoring"]["maximum_score"],
        )
        self.assertEqual(
            contract["scoring"]["pass_score"], contract["scoring"]["maximum_score"]
        )
        identities = set(contract["result_record"]["required_identity_fields"])
        self.assertIn("contract_sha256", identities)
        self.assertIn("harness_executable_sha256", identities)
        self.assertIn("profile_sha256", identities)
        observations = set(contract["result_record"]["required_observation_fields"])
        self.assertIn("artifact_sha256", observations)
        self.assertIn("hard_gate_results", observations)
        self.assertIn("functional_check_results", observations)
        tool_recording = contract["result_record"]["tool_call_recording"]
        self.assertIn("tool name", tool_recording)
        self.assertIn("Do not retain arguments", tool_recording)
        self.assertIn("generated file content", tool_recording)
        self.assertFalse(contract["privacy"]["store_reference_output"])
        self.assertFalse(contract["privacy"]["store_transcript"])


if __name__ == "__main__":
    unittest.main()
