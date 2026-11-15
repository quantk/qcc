#!/usr/bin/env python3
"""Turn a raw recording into a fixture that is safe to commit.

usage: redact.py {claude|acp|codex} RAW.ndjson FIXTURE.ndjson --work DIR [--home DIR]

A recording is NDJSON with one {"dir": "in"|"out"|"err", "line": "..."} per line that
crossed the agent's stdio (see record-claude.py and record-jsonrpc.py).

Redaction keeps the traffic a bridge reads and removes what identifies the machine
it was recorded on: absolute paths become /work and /home/user, stderr is dropped,
and bulky or personal fields the bridges never look at (tool catalogues, account and
rate-limit details, thinking signatures) are cut. It never invents traffic.
"""
import argparse, json, re, sys

parser = argparse.ArgumentParser()
parser.add_argument("dialect", choices=["claude", "acp", "codex"])
parser.add_argument("raw")
parser.add_argument("fixture")
parser.add_argument("--work", required=True, help="working directory used while recording")
parser.add_argument("--home", default=None)
args = parser.parse_args()

import os
home = args.home or os.path.expanduser("~")
user = os.path.basename(home)


def scrub_text(text):
    text = text.replace(args.work, "/work")
    # Tools print the path without its leading slash in titles.
    text = text.replace(args.work.lstrip("/"), "work")
    text = text.replace(home, "/home/user")
    return text


def scrub(value):
    if isinstance(value, str):
        return scrub_text(value)
    if isinstance(value, list):
        return [scrub(v) for v in value]
    if isinstance(value, dict):
        return {k: scrub(v) for k, v in value.items()}
    return value


def keep(obj, keys):
    return {k: obj[k] for k in keys if k in obj}


def claude(msg):
    kind = msg.get("type")
    if kind == "system" and msg.get("subtype") == "init":
        return keep(msg, ["type", "subtype", "session_id", "cwd", "model", "permissionMode",
                          "claude_code_version", "uuid"])
    if kind == "rate_limit_event":
        return {"type": "rate_limit_event", "rate_limit_info": {"status": msg["rate_limit_info"].get("status")}}
    if kind == "result":
        out = keep(msg, ["type", "subtype", "is_error", "result", "errors", "stop_reason",
                         "terminal_reason", "num_turns", "session_id", "duration_ms"])
        out["usage"] = keep(msg.get("usage", {}), ["input_tokens", "output_tokens",
                            "cache_creation_input_tokens", "cache_read_input_tokens"])
        out["modelUsage"] = {model: keep(usage, ["contextWindow"])
                             for model, usage in msg.get("modelUsage", {}).items()}
        return out
    if kind == "assistant":
        for block in msg["message"].get("content", []):
            if "signature" in block:
                block["signature"] = "redacted"
    if kind == "stream_event":
        delta = msg["event"].get("delta", {})
        if delta.get("type") == "signature_delta":
            delta["signature"] = "redacted"
        # Tool input streams in fragments that can split a path anywhere; the complete
        # input arrives again in the `assistant` message, which is what bridges read.
        if delta.get("type") == "input_json_delta":
            delta["partial_json"] = ""
        block = msg["event"].get("content_block", {})
        if "signature" in block:
            block["signature"] = "redacted" if block["signature"] else ""
    return msg


def acp(msg):
    result = msg.get("result")
    if isinstance(result, dict):
        for bulky in ("configOptions", "models", "modes"):
            result.pop(bulky, None)
    update = msg.get("params", {}).get("update") if isinstance(msg.get("params"), dict) else None
    if isinstance(update, dict) and update.get("sessionUpdate") == "available_commands_update":
        update["availableCommands"] = []
    if isinstance(update, dict) and update.get("sessionUpdate") == "config_option_update":
        update["configOptions"] = []
    return msg


CODEX_DROP = {"account/updated", "account/rateLimits/updated", "remoteControl/status/changed",
              "mcpServer/startupStatus/updated", "hook/started", "hook/completed"}


def codex(msg):
    if msg.get("method") in CODEX_DROP:
        return None
    if isinstance(msg.get("result"), dict) and "userAgent" in msg["result"]:
        msg["result"] = {"userAgent": "qcc/0.166.0", "codexHome": "/home/user/.codex",
                         "platformFamily": "unix", "platformOs": "linux"}
    return msg


dialect = {"claude": claude, "acp": acp, "codex": codex}[args.dialect]
kept = 0
with open(args.fixture, "w") as out:
    for raw in open(args.raw):
        entry = json.loads(raw)
        if entry["dir"] == "err":
            continue
        try:
            message = json.loads(entry["line"])
        except ValueError:
            continue
        if entry["dir"] == "out":
            message = dialect(message)
            if message is None:
                continue
        line = json.dumps(scrub(message), ensure_ascii=False, separators=(",", ":"))
        out.write(json.dumps({"dir": entry["dir"], "line": line}, ensure_ascii=False) + "\n")
        kept += 1

leaks = [word for word in (user, home, "claude-1000")
         if word and word in open(args.fixture).read()]
if leaks:
    sys.exit(f"{args.fixture}: still contains {leaks}")
print(f"{args.fixture}: {kept} lines")
