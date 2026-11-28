// Dev helper: talk to the app's WebView2 over the local DevTools protocol
// (only when started with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=PORT).
// Usage: node scripts/cdp.mjs PORT eval "<js expression>"
//        node scripts/cdp.mjs PORT console          (print console messages for 3 s)
//        node scripts/cdp.mjs PORT shot OUT.png     (screenshot of the page; Grabby cannot see WebView2 content)
//        node scripts/cdp.mjs PORT click "<css selector>"
const [port, cmd, arg] = process.argv.slice(2);
const pages = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const ws = new WebSocket(pages.find((p) => p.type === "page").webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
const send = (method, params = {}) =>
  new Promise((res) => {
    pending.set(++id, res);
    ws.send(JSON.stringify({ id, method, params }));
  });
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) pending.get(msg.id)(msg.result ?? msg.error);
  else if (msg.method === "Runtime.consoleAPICalled") console.log("console:", msg.params.args.map((a) => a.value ?? a.description).join(" "));
  else if (msg.method === "Runtime.exceptionThrown") console.log("exception:", msg.params.exceptionDetails.exception?.description);
  else if (msg.method === "Log.entryAdded") console.log("log:", msg.params.entry.level, msg.params.entry.text);
};
await new Promise((r) => (ws.onopen = r));
if (cmd === "eval") {
  const r = await send("Runtime.evaluate", { expression: arg, awaitPromise: true, returnByValue: true });
  console.log(JSON.stringify(r.result?.value ?? r, null, 1));
} else if (cmd === "shot") {
  const r = await send("Page.captureScreenshot", { format: "png" });
  (await import("node:fs")).writeFileSync(arg, Buffer.from(r.data, "base64"));
  console.log("saved", arg);
} else if (cmd === "click") {
  const r = await send("Runtime.evaluate", {
    expression: `(() => { const el = document.querySelector(${JSON.stringify(arg)}); if (!el) return "not found"; el.click(); return "clicked"; })()`,
    returnByValue: true,
  });
  console.log(r.result?.value);
} else if (cmd === "pdf") {
  // Print the page to a PDF (as the printer would get it): node scripts/cdp.mjs PORT pdf OUT.pdf
  const r = await send("Page.printToPDF", { preferCSSPageSize: true, printBackground: true });
  if (!r.data) { console.log(JSON.stringify(r)); process.exit(1); }
  (await import("node:fs")).writeFileSync(arg, Buffer.from(r.data, "base64"));
  console.log("saved", arg);
} else if (cmd === "console") {
  await send("Runtime.enable");
  await send("Log.enable");
  await send("Runtime.evaluate", { expression: "location.reload()" });
  await new Promise((r) => setTimeout(r, 3000));
}
ws.close();
