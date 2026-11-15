// The page talks to its plugin through the app: `qcc.action(name, value)` sends an
// action to the plugin's `Ui::action` and resolves with what it answered. That is
// the only way out of the frame: it has no network and no access to the app.
const qcc = (() => {
  const waiting = new Map();
  let next = 1;
  addEventListener("message", (event) => {
    const reply = event.data;
    if (event.source !== parent || !reply || reply.qcc !== 1) return;
    const pending = waiting.get(reply.id);
    if (!pending) return;
    waiting.delete(reply.id);
    if (reply.error === undefined) pending.resolve(reply.result);
    else pending.reject(new Error(reply.error));
  });
  return {
    action(action, value) {
      return new Promise((resolve, reject) => {
        const id = next++;
        waiting.set(id, { resolve, reject });
        parent.postMessage({ qcc: 1, id, action, value }, "*");
      });
    },
  };
})();

const canvas = document.getElementById("canvas");
const status = document.getElementById("status");
const context = canvas.getContext("2d");
let strokes = [];
let current = null;

function draw() {
  context.clearRect(0, 0, canvas.width, canvas.height);
  context.lineWidth = 3;
  context.lineCap = "round";
  context.lineJoin = "round";
  context.strokeStyle = getComputedStyle(document.body).color;
  for (const stroke of current ? [...strokes, current] : strokes) {
    context.beginPath();
    stroke.forEach(([x, y], index) => (index ? context.lineTo(x, y) : context.moveTo(x, y)));
    context.stroke();
  }
  status.textContent = `${strokes.length} stroke${strokes.length === 1 ? "" : "s"} saved`;
}

function point(event) {
  const box = canvas.getBoundingClientRect();
  const scale = canvas.width / box.width;
  return [Math.round((event.clientX - box.left) * scale), Math.round((event.clientY - box.top) * scale)];
}

canvas.addEventListener("pointerdown", (event) => {
  canvas.setPointerCapture(event.pointerId);
  current = [point(event)];
});
canvas.addEventListener("pointermove", (event) => {
  if (!current) return;
  current.push(point(event));
  draw();
});
canvas.addEventListener("pointerup", async () => {
  if (!current) return;
  strokes.push(current);
  current = null;
  draw();
  await qcc.action("save", JSON.stringify(strokes));
});

document.getElementById("clear").addEventListener("click", async () => {
  ({ strokes } = await qcc.action("clear"));
  strokes ??= [];
  draw();
});

qcc.action("load").then((state) => {
  strokes = Array.isArray(state.strokes) ? state.strokes : [];
  draw();
}, (error) => {
  status.textContent = String(error);
});
