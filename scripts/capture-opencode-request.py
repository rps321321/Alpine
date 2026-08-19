from __future__ import annotations

import argparse
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


class CaptureHandler(BaseHTTPRequestHandler):
    server_version = "LocalModelCapture/1"

    def do_GET(self) -> None:  # noqa: N802
        if self.path.endswith("/models"):
            body = json.dumps({"object": "list", "data": [{"id": "Qwen3.8-27B-ABLITERATED", "object": "model"}]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_error(404)

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        request = json.loads(raw)
        self.server.output.write_text(json.dumps(request, indent=2) + "\n", encoding="utf-8")  # type: ignore[attr-defined]
        chunks = [
            {"id": "capture", "object": "chat.completion.chunk", "created": 0, "model": "capture", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "CAPTURED"}, "finish_reason": None}]},
            {"id": "capture", "object": "chat.completion.chunk", "created": 0, "model": "capture", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]},
        ]
        body = "".join(f"data: {json.dumps(chunk, separators=(',', ':'))}\n\n" for chunk in chunks) + "data: [DONE]\n\n"
        encoded = body.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)
        threading.Thread(target=self.server.shutdown, daemon=True).start()

    def log_message(self, format: str, *args: object) -> None:
        return


def summarize(path: Path) -> dict[str, object]:
    request = json.loads(path.read_text(encoding="utf-8"))
    messages = request.get("messages", [])
    tools = request.get("tools", [])
    tool_rows = []
    for tool in tools:
        function = tool.get("function", {})
        encoded = json.dumps(tool, separators=(",", ":"), ensure_ascii=False)
        tool_rows.append({"name": function.get("name", "unknown"), "json_chars": len(encoded), "json_bytes": len(encoded.encode("utf-8"))})
    tool_rows.sort(key=lambda item: item["json_bytes"], reverse=True)
    message_rows = []
    for index, message in enumerate(messages):
        encoded = json.dumps(message, separators=(",", ":"), ensure_ascii=False)
        message_rows.append({"index": index, "role": message.get("role"), "json_chars": len(encoded), "json_bytes": len(encoded.encode("utf-8"))})
    return {
        "request_bytes": path.stat().st_size,
        "message_count": len(messages),
        "messages": message_rows,
        "tool_count": len(tools),
        "tools": tool_rows,
        "tool_json_bytes_total": sum(int(item["json_bytes"]) for item in tool_rows),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--summary", type=Path)
    parser.add_argument("--port", type=int, default=8199)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), CaptureHandler)
    server.output = args.output  # type: ignore[attr-defined]
    server.serve_forever()
    if args.summary:
        args.summary.write_text(json.dumps(summarize(args.output), indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
