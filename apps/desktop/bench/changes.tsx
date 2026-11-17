// A benchmark of the Changes tab and panel on a recorded diff: not part of the app.
// Served by Vite and driven by bench/run.mjs in headless Chrome.
import { Profiler, StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";

import type { Api } from "../src/api/api";
import type { Thread } from "../src/api/protocol.gen";
import { ChangesView } from "../src/modules/changes/ChangesView";
import { ChangesPanel } from "../src/modules/changes/ChangesPanel";
import { AppStore, provideStore } from "../src/state/store";
import "../src/styles.css";

const params = new URLSearchParams(location.search);
const which = params.get("view") ?? "tab";
const data = params.get("data") ?? "data";
const results: Record<string, unknown> = {};
(window as unknown as { results: typeof results }).results = results;

const [changes, diff, full] = await Promise.all([
  fetch(`./${data}/changes.json`).then((r) => r.json()),
  fetch(`./${data}/diff.json`).then((r) => r.json()),
  fetch(`./data/full.diff`).then((r) => r.text()),
]);
// The whole diff by file, to answer `paths` as the daemon does.
const byPath = new Map<string, string>();
for (const chunk of full.split(/^(?=diff --git )/m)) {
  const head = chunk.slice(0, chunk.indexOf("\n"));
  const rest = head.slice("diff --git ".length);
  const path = rest.slice((rest.length - 1) / 2 + 1).replace(/^b\//, "");
  byPath.set(path, chunk);
}
let bytes = 0;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
let calls = 0;
let answered = 0;
const api: Api = {
  async call(method: string, params: { paths?: string[] }) {
    calls++;
    if (method === "thread.changes") return changes;
    if (method === "thread.diff") {
      const paths: string[] = params?.paths ?? [];
      // As measured on the daemon: about 20 ms, and 4 more for each file.
      await sleep(20 + 4 * paths.length);
      answered ||= performance.now();
      const answer = paths.length ? { diff: paths.map((path) => byPath.get(path) ?? "").join("") } : diff;
      bytes += answer.diff.length;
      return answer;
    }
    if (method === "comment.list") return { comments: [] };
    throw new Error(method);
  },
  onNotification: () => () => {},
  onDaemonState: () => () => {},
  daemonState: async () => ({ status: "disconnected", error: "bench" }),
} as unknown as Api;
const store = new AppStore(api);
provideStore(store);

const thread = { id: "thr_bench", status: "idle", environment: null, cwd: "/tmp" } as unknown as Thread;
let commits = 0;
let renderMs = 0;
const onRender = (_id: string, _phase: string, actual: number) => {
  commits++;
  renderMs += actual;
};

const longTasks: number[] = [];
new PerformanceObserver((list) => list.getEntries().forEach((e) => longTasks.push(e.duration))).observe({ entryTypes: ["longtask"] });

let bump: () => void = () => {};
function Bench() {
  const [key, setKey] = useState(0);
  bump = () => setKey((k) => k + 1);
  return (
    <Profiler id="changes" onRender={onRender}>
      <div key={key} style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, overflow: which === "panel" ? "auto" : undefined }}>
        {which === "panel" ? <ChangesPanel thread={thread} /> : <ChangesView thread={thread} />}
      </div>
    </Profiler>
  );
}

const frame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
async function settle() {
  // Until nothing has been committed for a while: highlighting arrives late.
  let last = -1;
  let quiet = 0;
  while (quiet < 10) {
    await frame();
    if (commits === last) quiet++;
    else quiet = 0;
    last = commits;
  }
}

const t0 = performance.now();
createRoot(document.getElementById("root")!).render(<StrictMode><Bench /></StrictMode>);
// Wait for the data, then for the first paint after it.
while (!document.querySelector(".diff, .changed-files")) await new Promise((r) => setTimeout(r, 5));
await frame();
results.listShown = Math.round(performance.now() - t0);
// Jump to a file in the middle of the list, as the user would, and wait for its lines.
const target = document.querySelectorAll<HTMLButtonElement>(".changed-files button")[150]!;
const jumpPath = target.title;
const j0 = performance.now();
target.click();
const shownLines = () =>
  [...document.querySelectorAll<HTMLElement>("[title]")].some((el) => el.title === jumpPath && !el.closest(".changed-files")) &&
  document.body.textContent!.includes("@@ -");
while (!shownLines()) await new Promise((r) => setTimeout(r, 5));
await frame();
results.jumpToLines = Math.round(performance.now() - j0);
await settle();
results.settled = Math.round(performance.now() - t0);
results.mountRenderMs = Math.round(renderMs);
results.mountLongTasksMs = Math.round(longTasks.reduce((a, b) => a + b, 0));
results.maxLongTaskMs = Math.round(Math.max(0, ...longTasks));
results.domNodes = document.querySelectorAll("*").length;
results.bytesAfterMount = bytes;
results.rows = document.querySelectorAll("tr").length;

// A re-render with nothing new, as when a store update reaches the view.
renderMs = 0;
longTasks.length = 0;
const r0 = performance.now();
flushSync(() => store["set"]({ comments: { ...store.getState().comments } }));
await frame();
results.storeUpdateMs = Math.round(performance.now() - r0);

// Scroll through the body in steps and time the frames.
const body = [...document.querySelectorAll<HTMLElement>("#root *")].reduce((best, el) =>
  el.scrollHeight - el.clientHeight > best.scrollHeight - best.clientHeight ? el : best,
);
results.scrollHeight = body?.scrollHeight;
longTasks.length = 0;
const s0 = performance.now();
let worst = 0;
for (let i = 0; i < 40; i++) {
  const f0 = performance.now();
  body.scrollTop += 800;
  await frame();
  worst = Math.max(worst, performance.now() - f0);
}
results.scroll40FramesMs = Math.round(performance.now() - s0);
results.worstScrollFrameMs = Math.round(worst);
results.scrollTopEnd = body.scrollTop;
results.readingRightAfterScroll = (document.body.textContent!.match(/Reading the diff/g) ?? []).length;
await settle();
await new Promise((r) => setTimeout(r, 500));
results.readingAfterSettle = (document.body.textContent!.match(/Reading the diff/g) ?? []).length;
results.calls = calls;
results.bytesTotal = bytes;
results.domNodesAfterScroll = document.querySelectorAll("*").length;
results.highlightedSpans = document.querySelectorAll("[style*='--shiki-light']").length;
results.done = true;
