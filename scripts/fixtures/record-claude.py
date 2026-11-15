#!/usr/bin/env python3
"""Drive `claude -p` over stream-json and record both directions.

usage: rec_claude.py OUT.ndjson POLICY STEPS_JSON [claude args...]
POLICY: allow | deny   (answer to can_use_tool)
STEPS: JSON list; each step is {"send": <obj>} | {"wait": "<type>[:subtype]"} | {"sleep": secs}
       | {"interrupt": true}
"""
import json, subprocess, sys, threading, time, queue, uuid

out_path, policy, steps = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
args = ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json",
        "--verbose", "--include-partial-messages", "--permission-prompt-tool", "stdio"] + sys.argv[4:]
proc = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
log = open(out_path, "w")
lock = threading.Lock()
seen = queue.Queue()

def record(direction, line):
    with lock:
        log.write(json.dumps({"dir": direction, "line": line}) + "\n"); log.flush()

def send(obj):
    line = json.dumps(obj)
    record("in", line)
    proc.stdin.write(line + "\n"); proc.stdin.flush()

def reader():
    for line in proc.stdout:
        line = line.rstrip("\n")
        record("out", line)
        try: msg = json.loads(line)
        except Exception: continue
        if msg.get("type") == "control_request" and msg["request"].get("subtype") == "can_use_tool":
            if policy == "allow":
                resp = {"behavior": "allow", "updatedInput": msg["request"].get("input", {})}
            else:
                resp = {"behavior": "deny", "message": "The user declined this action."}
            send({"type": "control_response", "response": {"subtype": "success", "request_id": msg["request_id"], "response": resp}})
        seen.put(msg)
    seen.put(None)

threading.Thread(target=reader, daemon=True).start()
threading.Thread(target=lambda: [record("err", l.rstrip("\n")) for l in proc.stderr], daemon=True).start()

def wait(spec, timeout=120):
    kind, _, sub = spec.partition(":")
    deadline = time.time() + timeout
    while True:
        msg = seen.get(timeout=max(0.1, deadline - time.time()))
        if msg is None: raise SystemExit("process ended while waiting for " + spec)
        if msg.get("type") == kind and (not sub or msg.get("subtype") == sub or msg.get("event", {}).get("type") == sub):
            return msg

for step in steps:
    if "send" in step: send(step["send"])
    elif "wait" in step: wait(step["wait"])
    elif "sleep" in step: time.sleep(step["sleep"])
    elif "interrupt" in step:
        send({"type": "control_request", "request_id": "int-" + uuid.uuid4().hex[:8], "request": {"subtype": "interrupt"}})
proc.stdin.close()
try: proc.wait(timeout=30)
except subprocess.TimeoutExpired: proc.kill()
time.sleep(0.3)
print("exit", proc.returncode)
