from __future__ import annotations

import json
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
FOCUS_TIMER = REPO_ROOT / "benchmarks" / "experiments" / "focus-timer.json"
AGENT_ENGINE_BAKEOFF = REPO_ROOT / "config" / "agent-engine-bakeoff.json"
AGENT_ENGINE_FIXTURE = (
    REPO_ROOT / "benchmarks" / "agent-engine-bakeoff" / "public-v1" / "task.json"
)
AGENT_ENGINE_WORKER = REPO_ROOT / "scripts" / "agent-engine-bakeoff-worker.mjs"
EXPECTED_PROMPT = (
    "Use write exactly once create index.html. tiny offline Focus Timer, circular 25-min "
    "countdown, Start/Pause Reset, completed-session localStorage. HARD LIMIT 90 nonblank "
    "lines, 1200 words. no dependencies. Do not call write_file. reply DONE."
)


class HarnessBenchmarkContractTests(unittest.TestCase):
    def test_agent_engine_bakeoff_is_source_pinned_bounded_and_private(self) -> None:
        plan = json.loads(AGENT_ENGINE_BAKEOFF.read_text(encoding="utf-8"))

        self.assertEqual(plan["schema"], 1)
        self.assertEqual(plan["id"], "agent-engine-bakeoff-v1")
        self.assertEqual(
            [candidate["id"] for candidate in plan["candidates"]],
            [
                "opencode-process",
                "pi-sdk-core",
                "pi-process-rpc",
                "cline-agents",
            ],
        )
        for candidate in plan["candidates"]:
            source = candidate["source"]
            self.assertEqual(len(source["commit"]), 40)
            self.assertTrue(source["package_integrity"].startswith("sha512-"))
            self.assertIn(source["license"], {"MIT", "Apache-2.0"})
            self.assertTrue(candidate["smallest_missing_upstream_hook"])
            self.assertFalse(candidate["security"]["built_in_sandbox"])

        self.assertEqual(plan["budget"]["requests"], 24)
        self.assertEqual(plan["budget"]["max_event_queue"], 128)
        self.assertEqual(len(plan["required_scenarios"]), 11)
        self.assertEqual(plan["inputs"]["profile"], "stable-16k")
        self.assertEqual(plan["inputs"]["model_id"], "local-qwen")
        self.assertEqual(plan["inputs"]["temperature"], 0)
        self.assertEqual(
            plan["inputs"]["fixture"],
            "benchmarks/agent-engine-bakeoff/public-v1/task.json",
        )
        self.assertEqual(plan["recommendation"]["decision"], "no-go")
        self.assertEqual(
            [package["package"] for package in plan["supporting_packages"]],
            ["@earendil-works/pi-ai"],
        )
        self.assertTrue(
            plan["supporting_packages"][0]["package_integrity"].startswith("sha512-")
        )
        self.assertFalse(any(plan["privacy"].values()))
        published = AGENT_ENGINE_BAKEOFF.read_text(encoding="utf-8")
        self.assertNotIn("C:\\Users\\", published)
        self.assertNotIn("api_key", published.lower())
        self.assertNotIn('"prompt":', published.lower())
        self.assertTrue(AGENT_ENGINE_FIXTURE.is_file())
        worker = AGENT_ENGINE_WORKER.read_text(encoding="utf-8")
        self.assertIn('request.candidate === "opencode-process"', worker)
        self.assertIn('request.candidate === "pi-sdk-core"', worker)
        self.assertIn('request.candidate === "pi-process-rpc"', worker)
        self.assertIn('request.candidate === "cline-agents"', worker)
        self.assertIn('"no-exact-system-prompt-override"', worker)
        self.assertIn('"rpc-read-cannot-enforce-exact-path"', worker)
        self.assertIn('event.type === "turn_start"', worker)
        self.assertIn('event.type === "turn-started"', worker)
        self.assertIn("maxIterations: request.budget.requests", worker)
        self.assertIn("modelOptions: {", worker)
        self.assertIn("maxTokens: policy.max_output_tokens", worker)
        self.assertIn("temperature: policy.temperature", worker)
        self.assertNotIn("maxTokensPerTurn", worker)
        self.assertIn("failureFromReceipt", worker)
        self.assertIn("...receipt", worker)
        self.assertNotIn("budgetFailure(receipt.requests_used)", worker)
        self.assertNotIn("safeFailure(\"pi-sdk-scenario-not-observed\", receipt.requests_used)", worker)
        self.assertNotIn("safeFailure(\"cline-scenario-not-observed\", receipt.requests_used)", worker)
        self.assertNotIn('receipt.requests_used = 1', worker)
        self.assertNotIn("console.error", worker)

    def test_focus_timer_is_preserved_only_as_an_unimplemented_experiment(self) -> None:
        contract = json.loads(FOCUS_TIMER.read_text(encoding="utf-8"))

        self.assertEqual(contract["schema"], 1)
        self.assertEqual(contract["id"], "focus-timer-v1")
        self.assertEqual(contract["status"], "unimplemented-experiment")
        self.assertFalse(contract["qualification_evidence"])
        self.assertFalse((REPO_ROOT / "config" / "harness-benchmarks" / "focus-timer.json").exists())
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
