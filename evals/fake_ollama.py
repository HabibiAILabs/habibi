#!/usr/bin/env python3
"""Deterministic Ollama-compatible server used by eval tests; never contacts a model."""
import argparse, json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

EXPECTED = {
    "chat-delivery": "eval chat delivery ok",
    "cross-session-recall": "Indigo Finch",
    "tool-discovery": "discovery ok",
    "web-search": "search ok",
    "multiple-actions": "multiple ok",
    "mixed-delivery": "mixed ok",
    "action-failure": "recovered",
    "post-delivery-no-loop": "delivered once",
}


def tool(name, arguments, call_id):
    return {"id": call_id, "type": "function", "function": {"name": name.replace(".", "__"), "arguments": arguments}}


def current_event(body):
    for message in reversed(body.get("messages", [])):
        content = message.get("content")
        if not isinstance(content, str):
            continue
        try:
            value = json.loads(content)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and isinstance(value.get("current_event"), dict):
            return value["current_event"]
    return {}


def scripted_calls(fixture, event):
    event_type = event.get("event_type")
    payload = event.get("payload") or {}
    session = payload.get("session_id", "current")
    content = str(payload.get("content", "")).lower()
    if event_type in {"chat.session.started", "chat.message.created"}:
        if fixture == "cross-session-recall" and "recall" in content:
            return [tool("chat.search_messages", {"query": "Indigo Finch", "_habibi_delivery": "asap"}, "search")]
        if fixture == "tool-discovery":
            return [tool("habibi.tools.search", {"query": "evaluation visible marker"}, "discover")]
        if fixture == "web-search":
            return [tool("web-search.search", {"query": "Habibi Assistant", "count": 1}, "web")]
        if fixture == "multiple-actions":
            return [tool("eval.immediate", {"_habibi_delivery": "batch"}, "one"), tool("eval.immediate", {"_habibi_delivery": "batch"}, "two")]
        if fixture == "mixed-delivery":
            return [tool("eval.immediate", {"_habibi_delivery": "asap"}, "asap"), tool("eval.marker", {"_habibi_delivery": "batch"}, "batch"), tool("eval.fail", {"_habibi_delivery": "batch"}, "failure"), tool("eval.malformed", {"_habibi_delivery": "batch"}, "malformed")]
        if fixture == "action-failure":
            return [tool("eval.fail", {}, "failure")]
        return [tool("chat.send_message", {"session_id": session, "content": EXPECTED.get(fixture, "stored")}, "reply")]
    if event_type == "action.result.succeeded" and payload.get("tool") == "habibi.tools.search":
        return [tool("eval.marker", {}, "marker")]
    if event_type == "action.result.succeeded" and payload.get("tool") in {"eval.marker", "web-search.search"}:
        return [tool("chat.send_message", {"session_id": session, "content": EXPECTED[fixture]}, "reply")]
    if event_type == "action.result.succeeded" and payload.get("tool") == "chat.search_messages":
        return [tool("chat.send_message", {"session_id": session, "content": EXPECTED[fixture]}, "reply")]
    if event_type == "action.result.failed" and fixture == "action-failure":
        return [tool("chat.send_message", {"session_id": session, "content": EXPECTED[fixture]}, "reply")]
    if event_type == "actions.completed" and fixture in {"multiple-actions", "mixed-delivery"}:
        return [tool("chat.send_message", {"session_id": session, "content": EXPECTED[fixture]}, "reply")]
    return []


class Handler(BaseHTTPRequestHandler):
    fixture = "chat-delivery"

    def log_message(self, *_args):
        pass

    def json_response(self, value):
        encoded = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def do_GET(self):
        if self.path.startswith("/search"):
            self.json_response({"results": [{"title": "Habibi Assistant", "url": "https://example.test/habibi", "content": "Deterministic eval result", "engine": "eval"}], "unresponsive_engines": []})
            return
        self.send_error(404)

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        if self.path.rstrip("/") == "/api/show":
            self.json_response({"capabilities": ["tools"], "model_info": {}})
            return
        if self.path.rstrip("/") == "/api/chat":
            calls = scripted_calls(self.fixture, current_event(body))
            self.json_response({
                "model": "eval-model", "done": True,
                "message": {"role": "assistant", "content": "", "tool_calls": calls},
                "prompt_eval_count": 20, "eval_count": 10,
            })
            return
        self.send_error(404)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--fixture", required=True)
    args = parser.parse_args()
    Handler.fixture = args.fixture
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()

if __name__ == "__main__":
    main()
