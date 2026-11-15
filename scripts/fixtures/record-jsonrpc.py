#!/usr/bin/env python3
"""Drive a line-delimited JSON-RPC agent (ACP, codex app-server) and record both directions.

usage: rec_rpc.py OUT.ndjson STEPS_JSON -- cmd args...
Steps: {"request": {method, params}, "as": "name"}  send request, remember its id under name
       {"notify": {method, params}}
       {"wait_response": "name"}     wait for the response to a named request (stored for $refs)
       {"wait_method": "m"}          wait for an incoming request/notification with method m
       {"respond": "m", "result": {...}}  answer the last incoming request with method m
       {"sleep": s}
Strings of the form "$name.path.to.field" in params are replaced from stored responses/requests.
"""
import json, subprocess, sys, threading, time, queue

out_path, steps = sys.argv[1], json.loads(sys.argv[2])
cmd = sys.argv[sys.argv.index("--") + 1:]
proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
log = open(out_path, "w"); lock = threading.Lock(); seen = queue.Queue()
def record(d, line):
    with lock: log.write(json.dumps({"dir": d, "line": line}) + "\n"); log.flush()
def send(obj):
    line = json.dumps(obj); record("in", line); proc.stdin.write(line + "\n"); proc.stdin.flush()
def reader():
    for line in proc.stdout:
        line = line.rstrip("\n"); record("out", line)
        try: seen.put(json.loads(line))
        except Exception: pass
    seen.put(None)
threading.Thread(target=reader, daemon=True).start()
threading.Thread(target=lambda: [record("err", l.rstrip("\n")) for l in proc.stderr], daemon=True).start()

store, ids, backlog, next_id = {}, {}, [], [1]
def resolve(v):
    if isinstance(v, str) and v.startswith("$"):
        name, *path = v[1:].split(".")
        cur = store[name]
        for p in path: cur = cur[int(p)] if isinstance(cur, list) else cur[p]
        return cur
    if isinstance(v, dict): return {k: resolve(x) for k, x in v.items()}
    if isinstance(v, list): return [resolve(x) for x in v]
    return v
def wait(pred, what, timeout=180):
    for i, m in enumerate(backlog):
        if pred(m): return backlog.pop(i)
    deadline = time.time() + timeout
    while True:
        try: m = seen.get(timeout=max(0.1, deadline - time.time()))
        except queue.Empty: raise SystemExit("timeout waiting for " + what)
        if m is None: raise SystemExit("process ended waiting for " + what)
        if pred(m): return m
        backlog.append(m)
for step in steps:
    if "request" in step:
        rid = next_id[0]; next_id[0] += 1; ids[step.get("as", str(rid))] = rid
        send({"jsonrpc": "2.0", "id": rid, **resolve(step["request"])})
    elif "notify" in step: send({"jsonrpc": "2.0", **resolve(step["notify"])})
    elif "wait_response" in step:
        rid = ids[step["wait_response"]]
        m = wait(lambda m: "method" not in m and m.get("id") == rid, "response " + step["wait_response"])
        store[step["wait_response"]] = m.get("result", m.get("error"))
    elif "wait_method" in step:
        name = step["wait_method"]
        store[name.replace("/", "_")] = wait(lambda m: m.get("method") == name, name)
    elif "respond" in step:
        req = store[step["respond"].replace("/", "_")]
        send({"jsonrpc": "2.0", "id": req["id"], "result": resolve(step["result"])})
    elif "sleep" in step: time.sleep(step["sleep"])
time.sleep(0.5); proc.stdin.close()
try: proc.wait(timeout=10)
except subprocess.TimeoutExpired: proc.kill()
print("exit", proc.returncode)
