// Drives bench/changes.html in headless Chrome over the DevTools protocol and prints its results.
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [url] = process.argv.slice(2);
const port = 9333 + Math.floor(Math.random() * 500);
const chrome = spawn("google-chrome", ["--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${mkdtempSync(join(tmpdir(), "chrome-"))}`, "--window-size=1400,900", "about:blank"], { stdio: "ignore" });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let target;
for (let i = 0; i < 100 && !target; i++) {
  await sleep(100);
  try { target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === "page"); } catch {}
}
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0;
const pending = new Map();
ws.addEventListener("message", (e) => { const m = JSON.parse(e.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } else if (m.method === "Runtime.consoleAPICalled") console.error("console:", m.params.args.map((a) => a.value ?? a.description).join(" ").slice(0, 500)); else if (m.method === "Runtime.exceptionThrown") console.error(JSON.stringify(m.params.exceptionDetails).slice(0, 2000)); });
const send = (method, params = {}) => new Promise((r) => { pending.set(++id, r); ws.send(JSON.stringify({ id, method, params })); });
await send("Runtime.enable");
await send("Page.navigate", { url });
let results;
const limit = Number(process.env.BENCH_LIMIT ?? 1200);
for (let i = 0; i < limit; i++) {
  await sleep(250);
  const m = await send("Runtime.evaluate", { expression: "JSON.stringify(window.results ?? null)", returnByValue: true });
  results = JSON.parse(m.result?.result?.value ?? "null");
  if (results?.done) break;
}
if (!results?.done) { const m = await send("Runtime.evaluate", { expression: "document.body.innerText.slice(0, 1500) + ' | nodes ' + document.querySelectorAll('*').length", returnByValue: true }); console.error("page:", m.result?.result?.value); }
console.log(JSON.stringify(results));
chrome.kill();
process.exit(0);
