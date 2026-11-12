// The runtime-profiler browser journey for profiles/web-client.yaml: serves web/dist on
// loopback, opens the page's profile run (?frames=180: a scripted practice match, one tick per
// animation frame, as the native `--frames`) and waits for the frame times the page sets on
// `window.shooterProfile`. They go to $SHOOTER_WEB_PROFILE_OUT when it is set (the profiler
// keeps the trace, not the journey's output). See docs/foundation-data.md.
import { createServer } from "node:http";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const FRAMES = 180;
const RESULT_TIMEOUT_MS = 540_000;
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

export async function run({ page }) {
  await readFile(path.join(root, "shooter_client_bg.wasm")).catch(() => {
    throw new Error("web/dist has no wasm build: run scripts/build-web.sh first");
  });
  const server = await serve();
  try {
    const { port } = server.address();
    const logs = [];
    page.on("console", (message) => logs.push(message.text()));
    await page.goto(`http://127.0.0.1:${port}/?frames=${FRAMES}`, { waitUntil: "load" });
    const handle = await page.waitForFunction(() => Reflect.get(window, "shooterProfile"), null, {
      timeout: RESULT_TIMEOUT_MS,
      polling: 500,
    });
    const result = await handle.jsonValue();
    if (process.env.SHOOTER_WEB_PROFILE_OUT) {
      await writeFile(
        process.env.SHOOTER_WEB_PROFILE_OUT,
        JSON.stringify({ ...result, console: logs.slice(0, 50) }, null, 2) + "\n",
      );
    }
    if (result.error) throw new Error(`the profile run failed: ${result.error}`);
    if (result.frames !== FRAMES) throw new Error(`expected ${FRAMES} frames, got ${result.frames}`);
    return result;
  } finally {
    server.close();
    server.closeAllConnections();
  }
}
