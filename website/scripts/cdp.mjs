// Minimal Chrome DevTools Protocol helper for scripts/record-demo.mjs and og-image.mjs.
export async function open(port, { W, H, DPR, theme }) {
  const ver = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  const ws = new WebSocket(ver.webSocketDebuggerUrl);
  await new Promise((r) => ws.addEventListener("open", r, { once: true }));
  let id = 0; const pending = new Map(); const listeners = [];
  ws.addEventListener("message", (ev) => { const m = JSON.parse(ev.data); if (m.id && pending.has(m.id)) { const { res, rej } = pending.get(m.id); pending.delete(m.id); m.error ? rej(new Error(JSON.stringify(m.error))) : res(m.result); } else if (m.method) listeners.forEach((l) => l(m)); });
  const send = (method, params = {}, sessionId) => new Promise((res, rej) => { const i = ++id; pending.set(i, { res, rej }); ws.send(JSON.stringify({ id: i, method, params, sessionId })); });
  const { browserContextId } = await send("Target.createBrowserContext");
  const { targetId } = await send("Target.createTarget", { url: "about:blank", browserContextId });
  const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
  const s = (m, p) => send(m, p, sessionId);
  await s("Page.enable"); await s("Runtime.enable");
  const logs = [];
  listeners.push((m) => { if (m.sessionId !== sessionId) return; if (m.method === "Runtime.exceptionThrown") logs.push("EXC " + (m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text)); if (m.method === "Runtime.consoleAPICalled" && m.params.type === "error") logs.push("ERR " + m.params.args.map((a) => a.value ?? a.description).join(" ")); });
  const ev = async (expr) => { const r = await s("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 400)); return r.result.value; };
  await s("Emulation.setDeviceMetricsOverride", { width: W, height: H, deviceScaleFactor: DPR, mobile: false });
  await s("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const clickXY = async (x, y) => { await s("Input.dispatchMouseEvent", { type: "mouseMoved", x, y }); for (const type of ["mousePressed", "mouseReleased"]) await s("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1 }); };
  const click = async (expr) => { const box = await ev(`(()=>{const e=${expr}; if(!e) return null; const r=e.getBoundingClientRect(); return [r.x+r.width/2, r.y+r.height/2]})()`); if (!box) { console.log("missing", expr); return false; } await clickXY(box[0], box[1]); return true; };
  const key = async (k, { meta = false, code, vk } = {}) => { const mod = meta ? 4 : 0; await s("Input.dispatchKeyEvent", { type: "keyDown", key: k, code: code || k, windowsVirtualKeyCode: vk, modifiers: mod }); await s("Input.dispatchKeyEvent", { type: "keyUp", key: k, code: code || k, windowsVirtualKeyCode: vk, modifiers: mod }); };
  const shot = async (path) => { const { data } = await s("Page.captureScreenshot", { format: "png" }); (await import("node:fs")).writeFileSync(path, Buffer.from(data, "base64")); };
  const close = async () => { await send("Target.closeTarget", { targetId }); await send("Target.disposeBrowserContext", { browserContextId }); ws.close(); };
  return { s, send, ev, sleep, click, clickXY, key, shot, logs, listeners, sessionId, close };
}
