#!/usr/bin/env python3
"""A tiny MCP server over stdio for Waddle's tests.

Tools: echo (read-only), write_note (writes to $NOTES), fails (an error
result), crash (exits), slow (never answers in time), ask (pings the client
first). Tools are listed on two pages. Every request's method goes to stderr.
"""
import json
import os
import sys
import time

PAGE1 = [
    {"name": "echo", "description": "Repeats the text.", "inputSchema": {"$schema": "http://json-schema.org/draft-07/schema#", "type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}, "annotations": {"readOnlyHint": True}},
    {"name": "write_note", "description": "Saves a note.", "inputSchema": {"type": "object", "properties": {"note": {"type": "string"}}}, "annotations": {"readOnlyHint": False, "destructiveHint": False}},
    {"name": "fails", "description": "Always fails.", "inputSchema": {"type": "object"}},
]
PAGE2 = [
    {"name": "crash", "description": "Stops the server.", "inputSchema": {"type": "object"}},
    {"name": "slow", "description": "Takes forever.", "inputSchema": {"type": "object"}},
    {"name": "ask", "description": "Pings the client, then answers.", "inputSchema": {"type": "object"}},
]


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def text(t, error=False):
    r = {"content": [{"type": "text", "text": t}]}
    if error:
        r["isError"] = True
    return r


def main():
    for line in sys.stdin:
        msg = json.loads(line)
        method = msg.get("method")
        sys.stderr.write(f"got {method}\n")
        sys.stderr.flush()
        if "id" not in msg or method is None:
            continue  # notifications and replies to our ping
        mid = msg["id"]
        params = msg.get("params") or {}
        if method == "initialize":
            assert params["protocolVersion"] == "2025-11-25", params
            send({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}}})
        elif method == "tools/list":
            if params.get("cursor") == "p2":
                send({"jsonrpc": "2.0", "id": mid, "result": {"tools": PAGE2}})
            else:
                send({"jsonrpc": "2.0", "id": mid, "result": {"tools": PAGE1, "nextCursor": "p2"}})
        elif method == "tools/call":
            name = params.get("name")
            args = params.get("arguments") or {}
            if name == "echo":
                send({"jsonrpc": "2.0", "id": mid, "result": text("echo: " + args.get("text", ""))})
            elif name == "write_note":
                with open(os.environ["NOTES"], "a") as f:
                    f.write(args.get("note", "") + "\n")
                send({"jsonrpc": "2.0", "id": mid, "result": text("saved")})
            elif name == "fails":
                send({"jsonrpc": "2.0", "id": mid, "result": text("no such thing", error=True)})
            elif name == "crash":
                sys.exit(3)
            elif name == "slow":
                time.sleep(30)
            elif name == "ask":
                send({"jsonrpc": "2.0", "id": "srv-1", "method": "ping"})
                reply = json.loads(sys.stdin.readline())
                ok = reply.get("id") == "srv-1" and reply.get("result") == {}
                send({"jsonrpc": "2.0", "id": mid, "result": text("pong ok" if ok else f"bad pong {reply}")})
            else:
                send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32602, "message": f"unknown tool {name}"}})
        else:
            send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "unknown method"}})


main()
