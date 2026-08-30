#!/usr/bin/env python3
"""Run isolated Habibi eval fixtures through public HTTP APIs."""
import argparse, datetime as dt, hashlib, json, os, platform, shutil, socket, subprocess, sys, tempfile, time, urllib.error, urllib.request, uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = Path(__file__).with_name("fixtures.json")
ARTIFACT_ROOT = ROOT / ".benchmarks" / "habibi-evals"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request_json(url, method="GET", body=None, timeout=10):
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(url, data=data, method=method, headers={"content-type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.status, json.loads(response.read() or b"null")
    except urllib.error.HTTPError as error:
        payload = json.loads(error.read() or b"{}")
        raise RuntimeError(payload.get("error", f"HTTP {error.code}")) from error


def wait_ready(origin, process, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Habibi exited with {process.returncode}")
        try:
            request_json(origin + "/api/events?limit=1", timeout=1)
            return
        except (OSError, RuntimeError):
            time.sleep(.1)
    raise TimeoutError("Habibi did not become ready")


def stop(process):
    if not process or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(5)
    except subprocess.TimeoutExpired:
        process.kill(); process.wait()


def wait_for_trace_idle(origin, correlation_id, deadline, quiet_seconds=1.5):
    """Wait until a correlation has no active model/action span and stops changing."""
    signature = None
    quiet_since = None
    latest = None
    while time.monotonic() < deadline:
        _, latest = request_json(origin + "/api/trace?correlation_id=" + correlation_id)
        logs = [item.get("record", item) for item in latest.get("logs", [])]
        events = [item.get("record", item) for item in latest.get("events", [])]
        model_started = sum(item.get("name") == "model.invocation.started" for item in logs)
        model_finished = sum(item.get("name") in ("model.invocation.completed", "model.invocation.failed") for item in logs)
        action_started = sum(item.get("name") == "action.execution.started" for item in logs)
        action_finished = sum(item.get("name") == "action.execution.completed" for item in logs)
        current = (len(events), len(logs), max((item.get("sequence", 0) for item in events), default=0),
                   max((item.get("sequence", 0) for item in logs), default=0))
        busy = model_started != model_finished or action_started != action_finished
        if current != signature or busy:
            signature = current
            quiet_since = None
        elif quiet_since is None:
            quiet_since = time.monotonic()
        elif time.monotonic() - quiet_since >= quiet_seconds:
            return latest
        time.sleep(.2)
    return latest


def copy_extensions(destination):
    destination.mkdir()
    for name in ("chat", "web-search"):
        shutil.copytree(ROOT / "extensions" / name, destination / name)
    shutil.copytree(ROOT / "evals/extensions/eval-fixtures", destination / "eval-fixtures")


def hash_path(path):
    digest = hashlib.sha256()
    files = [path] if path.is_file() else sorted(item for item in path.rglob("*") if item.is_file())
    for item in files:
        digest.update(str(item.relative_to(ROOT) if item.is_relative_to(ROOT) else item.name).encode())
        digest.update(item.read_bytes())
    return digest.hexdigest()


def trace_metrics(trace):
    logs = [item.get("record", item) for item in trace.get("logs", [])]
    events = [item.get("record", item) for item in trace.get("events", [])]
    invocations = [item for item in logs if item.get("name") == "model.invocation.completed"]
    actions = []
    groups = {}
    for event in events:
        payload = event.get("payload") or {}
        if event.get("event_type") == "action.requested":
            groups.setdefault(payload.get("group_id"), []).append(event)
        if event.get("event_type", "").startswith("action.result."):
            actions.append({
                "event_id": event.get("id"), "sequence": event.get("sequence"),
                "tool": payload.get("tool"), "status": event["event_type"].removeprefix("action.result."),
                "delivery_mode": payload.get("delivery"), "arguments": None,
                "result": payload.get("result"), "error": payload.get("error"),
                "action_group_id": payload.get("group_id"), "index": payload.get("index"),
            })
    usage = {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0, "total_tokens": 0}
    cost = 0.0
    cost_known = True
    for log in invocations:
        payload = log.get("payload") or {}
        for key, value in (payload.get("usage") or {}).items():
            if key in usage and isinstance(value, int): usage[key] += value
        estimated = payload.get("estimated_cost")
        if estimated is None:
            cost_known = False
        else:
            cost += float(estimated["total_usd"])
    return {
        "actions": actions, "groups": groups, "model_invocations": len(invocations),
        "usage": usage, "estimated_cost_usd": cost, "cost_known": cost_known,
        "brave_searches": sum(action["tool"] == "web-search.search" and
                              isinstance(action.get("result"), dict) and action["result"].get("provider") == "brave"
                              for action in actions),
    }


def assertion(name, expected, actual, passed=None):
    return {"name": name, "expected": expected, "actual": actual, "pass": expected == actual if passed is None else passed}


def evaluate(fixture, messages, trace, duration):
    metrics = trace_metrics(trace)
    assistants = [message for message in messages if message.get("role") == "assistant"]
    actual = assistants[-1].get("content") if assistants else None
    tools = [action["tool"] for action in metrics["actions"]]
    group_sizes = [len(items) for items in metrics["groups"].values()]
    assertions = [
        assertion("assistant_exact", fixture["expected"], actual),
        assertion("required_tools", fixture["required_tools"], tools,
                  all(tool in tools for tool in fixture["required_tools"])),
        assertion("max_model_invocations", fixture["max_model_invocations"], metrics["model_invocations"],
                  metrics["model_invocations"] <= fixture["max_model_invocations"]),
        assertion("max_brave_searches", fixture["max_brave_searches"], metrics["brave_searches"],
                  metrics["brave_searches"] <= fixture["max_brave_searches"]),
        assertion("trace_not_truncated", False, bool(trace.get("truncated"))),
    ]
    if "minimum_group_size" in fixture:
        assertions.append(assertion("minimum_group_size", fixture["minimum_group_size"], max(group_sizes, default=0),
                                    max(group_sizes, default=0) >= fixture["minimum_group_size"]))
    if fixture.get("require_failure"):
        failures = sum(action["status"] == "failed" for action in metrics["actions"])
        assertions.append(assertion("action_failure_observed", True, failures > 0))
    if fixture.get("required_delivery_modes"):
        modes = {action["delivery_mode"] for action in metrics["actions"]}
        required = set(fixture["required_delivery_modes"])
        assertions.append(assertion("delivery_modes", sorted(required), sorted(modes), required <= modes))
    return {
        "fixture_id": fixture["id"], "pass": all(item["pass"] for item in assertions),
        "assertions": assertions, "expected_message": fixture["expected"], "actual_message": actual,
        "duration_ms": round(duration * 1000), "trace": trace, **metrics,
    }


def merge_trace_metrics(result, trace):
    """Include setup-event work in fixture-wide usage and budget accounting."""
    metrics = trace_metrics(trace)
    result["model_invocations"] += metrics["model_invocations"]
    for key, value in metrics["usage"].items():
        result["usage"][key] += value
    result["estimated_cost_usd"] += metrics["estimated_cost_usd"]
    result["cost_known"] = result["cost_known"] and metrics["cost_known"]
    result["brave_searches"] += metrics["brave_searches"]
    result["actions"].extend(metrics["actions"])
    for group_id, events in metrics["groups"].items():
        result["groups"].setdefault(group_id, []).extend(events)


def public_result(result):
    return {
        "fixture_id": result["fixture_id"], "pass": result["pass"],
        "assertions": [{"name": item["name"], "pass": item["pass"]} for item in result["assertions"]],
        "duration_ms": result["duration_ms"], "model_invocations": result["model_invocations"],
        "usage": result["usage"], "estimated_cost_usd": result["estimated_cost_usd"],
        "cost_known": result["cost_known"], "brave_searches": result["brave_searches"],
        "actions": [{key: action.get(key) for key in
            ("event_id", "sequence", "tool", "status", "delivery_mode", "action_group_id", "index")}
            for action in result["actions"]],
    }


def run_fixture(args, fixture, run_root):
    case_root = run_root / fixture["id"]
    case_root.mkdir(parents=True)
    extensions = case_root / "extensions"; copy_extensions(extensions)
    bind_port = free_port(); origin = f"http://127.0.0.1:{bind_port}"
    fake = None
    env = os.environ.copy()
    env.update({
        "HABIBI_DB": str(case_root / "habibi.db"), "HABIBI_BIND": f"127.0.0.1:{bind_port}",
        "HABIBI_EXTENSIONS_DIR": str(extensions), "HABIBI_STUDIO_DIR": str(case_root / "drafts"),
        "HABIBI_MODEL_PROVIDER": args.provider, "HABIBI_MODEL": args.model,
    })
    if not args.live:
        fake_port = free_port()
        fake = subprocess.Popen([sys.executable, str(ROOT / "evals/fake_ollama.py"), "--port", str(fake_port), "--fixture", fixture["id"]],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        fake_origin = f"http://127.0.0.1:{fake_port}"
        for _ in range(50):
            try:
                request_json(fake_origin + "/api/show", "POST", {"model": "eval-model"}, timeout=.2)
                break
            except OSError:
                time.sleep(.05)
        else:
            raise RuntimeError("fake Ollama did not become ready")
        env.update({"HABIBI_MODEL_PROVIDER": "ollama", "HABIBI_MODEL": "eval-model", "HABIBI_OLLAMA_URL": fake_origin,
                    "HABIBI_SEARCH_PROVIDER": "searxng", "HABIBI_SEARXNG_URL": fake_origin + "/search"})
    auth = os.environ.get("HABIBI_AUTH_FILE")
    if auth and Path(auth).exists():
        isolated_auth = case_root / "auth.json"; shutil.copy2(auth, isolated_auth); env["HABIBI_AUTH_FILE"] = str(isolated_auth)
    log = (case_root / "habibi.log").open("wb")
    process = subprocess.Popen([str(args.binary)], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
    started = time.monotonic()
    try:
        wait_ready(origin, process)
        if args.live and args.provider == "openai-codex":
            _, models = request_json(origin + "/api/models")
            priced = any(item.get("provider") == "openai-codex" and item.get("id") == args.model
                         and item.get("pricing") for item in models["catalog"]["models"])
            if not priced:
                raise RuntimeError(f"live model {args.model!r} has no catalog pricing; refusing priced run")
        setup_correlations = []
        for setup in fixture.get("setup", []):
            _, prior = request_json(origin + "/extensions/chat/api/sessions", "POST", {"request_id": str(uuid.uuid4()), "title": "Prior", "first_message": setup})
            setup_correlations.append(prior["correlation_id"])
            time.sleep(1)
        session_status, session = request_json(origin + "/extensions/chat/api/sessions", "POST", {"request_id": str(uuid.uuid4()), "title": fixture["id"]})
        if session_status != 202: raise RuntimeError(f"event-producing session route returned {session_status}, expected 202")
        session_id = session["id"]
        message_id = str(uuid.uuid4())
        message_body = {"message_id": message_id, "content": fixture["prompt"]}
        message_status, accepted = request_json(origin + f"/extensions/chat/api/sessions/{session_id}/messages", "POST", message_body)
        if message_status != 202: raise RuntimeError(f"event-producing message route returned {message_status}, expected 202")
        if fixture["id"] == "chat-delivery":
            retry_status, retried = request_json(origin + f"/extensions/chat/api/sessions/{session_id}/messages", "POST", message_body)
            if retry_status != 202 or retried["event_id"] != accepted["event_id"] or retried["correlation_id"] != accepted["correlation_id"]:
                raise RuntimeError("idempotent Chat retry did not return the original acceptance")
        deadline = time.monotonic() + args.timeout
        messages = []
        while time.monotonic() < deadline:
            _, history = request_json(origin + f"/extensions/chat/api/sessions/{session_id}/messages")
            messages = history["messages"]
            if any(message.get("role") == "assistant" for message in messages):
                break
            time.sleep(.2)
        trace = wait_for_trace_idle(origin, accepted["correlation_id"], deadline)
        _, history = request_json(origin + f"/extensions/chat/api/sessions/{session_id}/messages")
        messages = history["messages"]
        if trace is None:
            _, trace = request_json(origin + "/api/trace?correlation_id=" + accepted["correlation_id"])
        result = evaluate(fixture, messages, trace, time.monotonic() - started)
        setup_traces = []
        for correlation_id in setup_correlations:
            _, setup_trace = request_json(origin + "/api/trace?correlation_id=" + correlation_id)
            setup_traces.append(setup_trace)
            merge_trace_metrics(result, setup_trace)
        raw_result = {**result, "setup_traces": setup_traces}
        (case_root / "raw-result.json").write_text(json.dumps(raw_result, indent=2))
        return public_result(result)
    finally:
        stop(process); stop(fake); log.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--live", action="store_true", help="allow real provider/search requests")
    parser.add_argument("--provider", default="ollama", choices=["ollama", "openai-codex"])
    parser.add_argument("--model", default="gemma4")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/habibi")
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--max-cost-usd", type=float, default=2.0)
    parser.add_argument("--max-brave-searches", type=int, default=20)
    parser.add_argument("--reserve-per-fixture-usd", type=float, default=.20)
    parser.add_argument("--fixture", action="append")
    parser.add_argument("--allow-partial", action="store_true",
                        help="permit a successful report when a budget intentionally skips fixtures")
    args = parser.parse_args()
    if args.max_cost_usd < 0: raise SystemExit("cost ceiling must be non-negative")
    if args.reserve_per_fixture_usd <= 0: raise SystemExit("per-fixture reservation must be positive")
    if args.provider == "openai-codex" and args.max_cost_usd > 2: raise SystemExit("OpenAI cost ceiling cannot exceed $2")
    if not 0 <= args.max_brave_searches <= 20: raise SystemExit("Brave search ceiling must be between 0 and 20")
    if not args.binary.exists(): raise SystemExit(f"build Habibi first: {args.binary}")
    definitions = json.loads(FIXTURES.read_text())["fixtures"]
    if args.fixture: definitions = [item for item in definitions if item["id"] in args.fixture]
    run_id = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_root = ARTIFACT_ROOT / run_id; run_root.mkdir(parents=True)
    results = []; total_cost = 0.0; brave = 0; stop_reason = None
    for fixture in definitions:
        if args.live and args.provider == "openai-codex" and total_cost + args.reserve_per_fixture_usd > args.max_cost_usd:
            stop_reason = "cost reservation exhausted"; break
        if brave + fixture["max_brave_searches"] > args.max_brave_searches:
            stop_reason = "Brave search allowance exhausted"; break
        result = run_fixture(args, fixture, run_root); results.append(result)
        total_cost += result["estimated_cost_usd"]; brave += result["brave_searches"]
        if args.live and args.provider == "openai-codex" and not result["cost_known"]:
            result["pass"] = False; stop_reason = "provider cost was unknown"; break
        if total_cost > args.max_cost_usd:
            result["pass"] = False; stop_reason = "observed cost exceeded ceiling"; break
        if brave > args.max_brave_searches:
            result["pass"] = False; stop_reason = "observed Brave searches exceeded ceiling"; break
    executed = {result["fixture_id"] for result in results}
    skipped = [fixture["id"] for fixture in definitions if fixture["id"] not in executed]
    complete = not skipped and stop_reason is None
    provenance_inputs = [FIXTURES, ROOT / "evals/run.py", ROOT / "evals/report.py",
                         ROOT / "evals/fake_ollama.py", ROOT / "evals/extensions/eval-fixtures",
                         ROOT / "extensions/chat", ROOT / "extensions/web-search", ROOT / "model-catalog.json"]
    input_hashes = {str(path.relative_to(ROOT)): hash_path(path) for path in provenance_inputs}
    binary_hash = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    git_dirty = bool(subprocess.run(["git", "status", "--porcelain"], cwd=ROOT, text=True, capture_output=True).stdout)
    extension_inputs_tracked = bool(subprocess.run(
        ["git", "ls-files", "extensions/chat", "extensions/web-search"], cwd=ROOT,
        text=True, capture_output=True).stdout.strip())
    reproducible = not git_dirty and extension_inputs_tracked
    report = {
        "schema_version": 1, "run_id": run_id, "kind": "live" if args.live else "deterministic",
        "provider": args.provider if args.live else "fake-ollama", "model": args.model if args.live else "eval-model",
        "complete": complete, "planned_fixtures": [item["id"] for item in definitions],
        "executed_fixtures": [item["fixture_id"] for item in results], "skipped_fixtures": skipped,
        "stop_reason": stop_reason, "budget_kind": "soft-observed-with-preflight-reservation",
        "estimated_cost_usd": total_cost, "brave_searches": brave, "results": results,
        "reproducibility": {"command": sys.argv, "platform": platform.platform(), "python": sys.version,
                            "binary_sha256": binary_hash, "input_sha256": input_hashes,
                            "git_sha": subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, capture_output=True).stdout.strip(),
                            "git_dirty": git_dirty, "extension_inputs_tracked": extension_inputs_tracked,
                            "reproducible": reproducible,
                            "note": None if reproducible else "Hashes identify exact inputs, but dirty or ignored extension sources prevent reconstruction from the Git SHA alone."},
    }
    output = ROOT / "evals/reports" / f"{run_id}.json"; output.write_text(json.dumps(report, indent=2))
    html_output = output.with_suffix(".html")
    subprocess.run([sys.executable, str(ROOT / "evals/report.py"), str(output), "--output", str(html_output)], check=True)
    print(output); print(html_output)
    success = bool(results) and all(result["pass"] for result in results) and (complete or args.allow_partial)
    raise SystemExit(0 if success else 1)

if __name__ == "__main__": main()
