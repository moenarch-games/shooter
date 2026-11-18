// Browser (WebGPU) captures: serves web/dist on loopback, opens the page's profile run
// (?frames=<n>&map=<id>: a scripted practice match, one tick per animation frame, the local
// fighter played by the replays' scripted person 0) in headless Chromium at 1280x720, waits
// for the run to finish and screenshots the page (its last frame stays on the canvas) as a
// PNG. The run is deterministic: the same map and frame count draw the same frame. The
// result names the cover broken by then (shooter#60).
//
// Usage (after scripts/build-web.sh): node scripts/capture-web.mjs OUT MAP:FRAMES[:NAME] ...
// e.g. node scripts/capture-web.mjs docs/captures/53 proving-cell:120:browser-match
// writes docs/captures/53/browser-match.png.
//
// WebGPU runs on the machine's GPU (Vulkan). SHOOTER_CAPTURE_SWIFTSHADER=1 uses the profile
// lane's SwiftShader adapter (profiles/web-client.yaml) instead: it renders, but its frames
// do not reach the page's screenshot (the canvas comes out blank), so it only checks the run.
// See docs/foundation-data.md.
import { createServer } from "node:http";
import { mkdir, readFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";

const RESULT_TIMEOUT_MS = 300_000;
const root = path.resolve("web/dist");
const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
};

function serve() {
  const server = createServer(async (request, response) => {
    const name = new URL(request.url ?? "/", "http://localhost").pathname;
    const file = path.join(root, name === "/" ? "index.html" : name);
    if (!file.startsWith(root + path.sep)) {
      response.writeHead(403).end();
      return;
    }
    try {
      const body = await readFile(file);
      response.writeHead(200, {
        "content-type": types[path.extname(file)] ?? "application/octet-stream",
      });
      response.end(body);
    } catch {
      response.writeHead(404).end();
    }
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve(server));
  });
}

const [out, ...shots] = process.argv.slice(2);
if (!out || shots.length === 0) {
  console.error("usage: node scripts/capture-web.mjs OUT MAP:FRAMES[:NAME] ...");
  process.exit(2);
}
await readFile(path.join(root, "shooter_client_bg.wasm")).catch(() => {
  console.error("web/dist has no wasm build: run scripts/build-web.sh first");
  process.exit(2);
});
await mkdir(out, { recursive: true });

const args = ["--enable-unsafe-webgpu", "--disable-gpu-watchdog"];
if (process.env.SHOOTER_CAPTURE_SWIFTSHADER) args.push("--use-webgpu-adapter=swiftshader");
else args.push("--enable-features=Vulkan", "--use-angle=vulkan");
const browser = await chromium.launch({ args });
const server = await serve();
let failed = false;
try {
  const { port } = server.address();
  for (const shot of shots) {
    const [map, frames, name = `browser-${map}-${frames}`] = shot.split(":");
    const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
    const logs = [];
    page.on("console", (message) => logs.push(message.text()));
    await page.goto(`http://127.0.0.1:${port}/?frames=${frames}&map=${map}`, { waitUntil: "load" });
    const result = await page
      .waitForFunction(() => Reflect.get(window, "shooterProfile"), null, {
        timeout: RESULT_TIMEOUT_MS,
        polling: 250,
      })
      .then((handle) => handle.jsonValue())
      .catch((error) => ({ error: String(error.message ?? error).split("\n")[0] }));
    if (result.error) {
      failed = true;
      console.error(`${shot}: ${result.error}\n${logs.slice(-20).join("\n")}`);
    } else {
      const file = path.join(out, `${name}.png`);
      await page.screenshot({ path: file });
      const adapter = await page.evaluate(async () => {
        const info = (await navigator.gpu?.requestAdapter())?.info;
        return info ? `${info.vendor} ${info.architecture} ${info.description}`.trim() : "?";
      });
      const broken = result.broken?.length ? `, broken cover: ${result.broken.join(" ")}` : "";
      console.log(`${file}: ${result.frames} frames on ${result.map}${broken} (${adapter})`);
    }
    await page.close();
  }
} finally {
  await browser.close();
  server.close();
  server.closeAllConnections();
}
process.exit(failed ? 1 : 0);
