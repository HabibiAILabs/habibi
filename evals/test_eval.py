import importlib.util, json, tempfile, unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / f"{name}.py")
    value = importlib.util.module_from_spec(spec); spec.loader.exec_module(value)
    return value


class EvalContractTests(unittest.TestCase):
    def test_all_required_live_fixtures_have_budgets_and_assertions(self):
        fixtures = json.loads((ROOT / "fixtures.json").read_text())["fixtures"]
        self.assertEqual({item["id"] for item in fixtures}, {
            "chat-delivery", "cross-session-recall", "tool-discovery", "web-search",
            "multiple-actions", "mixed-delivery", "action-failure", "post-delivery-no-loop",
        })
        self.assertTrue(all(item["live"] and item["max_brave_searches"] <= 20 for item in fixtures))
        self.assertLessEqual(sum(item["max_brave_searches"] for item in fixtures), 20)

    def test_report_escapes_trace_content_and_has_no_external_assets(self):
        report = module("report")
        output = report.render({"run_id": "test", "provider": "fake", "model": "fake",
            "results": [{"fixture_id": "<fixture>", "pass": False,
                         "assertions": [{"name": "unsafe<script>", "actual": "<x>", "pass": False}]}]})
        self.assertNotIn("<script>", output)
        self.assertIn("&lt;fixture&gt;", output)
        self.assertNotIn("src=", output)

    def test_public_result_removes_prompts_results_and_trace(self):
        runner = module("run")
        public = runner.public_result({
            "fixture_id": "case", "pass": True,
            "assertions": [{"name": "secret", "expected": "prompt", "actual": "reply", "pass": True}],
            "duration_ms": 1, "model_invocations": 1, "usage": {}, "estimated_cost_usd": 0,
            "cost_known": True, "brave_searches": 0, "trace": {"provider_response": "secret"},
            "actions": [{"event_id": "e", "sequence": 1, "tool": "tool", "status": "succeeded",
                         "delivery_mode": "asap", "action_group_id": "g", "index": 0,
                         "result": {"private": "value"}, "error": None}],
        })
        encoded = json.dumps(public)
        self.assertNotIn("prompt", encoded)
        self.assertNotIn("reply", encoded)
        self.assertNotIn("provider_response", encoded)
        self.assertNotIn("value", encoded)

    def test_setup_trace_metrics_are_included_in_fixture_cost(self):
        runner = module("run")
        result = {
            "model_invocations": 1,
            "usage": {"input": 2, "output": 3, "cache_read": 0, "cache_write": 0, "total_tokens": 5},
            "estimated_cost_usd": .10,
            "cost_known": True,
            "brave_searches": 0,
            "actions": [],
            "groups": {},
        }
        setup_trace = {"events": [], "logs": [{"record": {
            "name": "model.invocation.completed",
            "payload": {
                "usage": {"input": 7, "output": 11, "total_tokens": 18},
                "estimated_cost": {"total_usd": .25},
            },
        }}]}
        runner.merge_trace_metrics(result, setup_trace)
        self.assertEqual(result["model_invocations"], 2)
        self.assertEqual(result["usage"]["total_tokens"], 23)
        self.assertAlmostEqual(result["estimated_cost_usd"], .35)
        self.assertTrue(result["cost_known"])

    def test_delivery_assertions_accept_mixed_modes_and_failure(self):
        runner = module("run")
        fixture = {"id": "mixed", "expected": "ok", "required_tools": ["a", "b"],
                   "max_model_invocations": 2, "max_brave_searches": 0,
                   "minimum_group_size": 2, "required_delivery_modes": ["asap", "batch"],
                   "require_failure": True}
        events = [
            {"record": {"event_type": "action.requested", "payload": {"group_id": "g"}}},
            {"record": {"event_type": "action.requested", "payload": {"group_id": "g"}}},
            {"record": {"id": "1", "sequence": 1, "event_type": "action.result.succeeded", "payload": {"tool": "a", "delivery": "asap", "group_id": "g"}}},
            {"record": {"id": "2", "sequence": 2, "event_type": "action.result.failed", "payload": {"tool": "b", "delivery": "batch", "group_id": "g"}}},
        ]
        result = runner.evaluate(fixture, [{"role": "assistant", "content": "ok"}],
                                 {"events": events, "logs": [], "truncated": False}, .01)
        self.assertTrue(result["pass"], result["assertions"])


if __name__ == "__main__": unittest.main()
