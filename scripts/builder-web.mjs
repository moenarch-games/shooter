// The robot builder clicked through in a real browser (shooter#124): serves web/dist on loopback,
// opens the page (practice, the builder before the match) in headless Chromium at 1280x720 on
// WebGPU and drives it with the mouse and keys as a person would: a part taken from the
// catalogue, a socket picked under the pointer, the part turned about each axis in hand (R, T,
// F; G steps to an eighth and a degree), placed, turned again on its socket, undone and redone
// back to the exact builds, a refused placement, a cancelled drag, and Ready, which starts the
// match. Every step checks what the page's builder readout (`window.shooterBuilder`, the
// builder's own state: the build's bytes, the part in hand, the placement under the pointer and
// its verdict, where sockets and buttons are) says against what the step should do, and
// screenshots the page into OUT.
//
// Usage (after scripts/build-web.sh): node scripts/builder-web.mjs OUT
// e.g. node scripts/builder-web.mjs docs/captures/124
// It exits non-zero on the first step that does not do what it should.
//
// WebGPU runs on the machine's GPU (Vulkan), as scripts/capture-web.mjs; `?renderer=lite` (the
// WebGL 2 renderer) by SHOOTER_BUILDER_RENDERER=lite.
import { createServer } from "node:http";
import { mkdir, readFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";

const root = path.resolve("web/dist");
const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
};
const QUARTER = 16_384;
const EIGHTH = 8_192;

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

const [out] = process.argv.slice(2);
if (!out) {
  console.error("usage: node scripts/builder-web.mjs OUT");
  process.exit(2);
}
await readFile(path.join(root, "shooter_client_bg.wasm")).catch(() => {
  console.error("web/dist has no wasm build: run scripts/build-web.sh first");
  process.exit(2);
});
await mkdir(out, { recursive: true });

const browser = await chromium.launch({
  args: [
    "--enable-unsafe-webgpu",
    "--disable-gpu-watchdog",
    "--enable-features=Vulkan",
    "--use-angle=vulkan",
  ],
});
const server = await serve();
const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
const logs = [];
page.on("console", (message) => logs.push(message.text()));
page.on("pageerror", (error) => logs.push(`page error: ${error.message}`));

let shots = 0;
function check(condition, what, state) {
  if (!condition) {
    const detail = state ? JSON.stringify({ ...state, sockets: undefined, targets: undefined }) : "";
    throw new Error(`${what}\n${detail}`);
  }
  console.log(`ok: ${what}`);
}

/** The builder's readout. */
const read = () => page.evaluate(() => Reflect.get(window, "shooterBuilder") ?? null);

/** The readout once `ready(state)` holds (a few frames after an input). */
async function until(ready, what) {
  const deadline = Date.now() + 15_000;
  let state = await read();
  while (!(state && ready(state))) {
    if (Date.now() > deadline) check(false, `timed out: ${what}`, state);
    await page.waitForTimeout(50);
    state = await read();
  }
  check(true, what);
  return state;
}

/** The readout after the page has drawn a few more frames. */
async function settle() {
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(done)))),
  );
  return read();
}

async function shoot(name) {
  shots += 1;
  const file = path.join(out, `browser-${String(shots).padStart(2, "0")}-${name}.png`);
  await page.screenshot({ path: file });
  console.log(`   ${file}`);
}

/** The pointer in CSS pixels at a readout position (the window's device pixels). */
async function cssAt([x, y]) {
  const ratio = await page.evaluate(() => devicePixelRatio);
  return [x / ratio, y / ratio];
}

async function hover(at) {
  const [x, y] = await cssAt(at);
  await page.mouse.move(x, y, { steps: 4 });
}

async function click(at) {
  await hover(at);
  await page.mouse.down();
  await page.mouse.up();
}

const target = (state, name) => state.targets.find((item) => item.name === name);
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

try {
  const { port } = server.address();
  const renderer = process.env.SHOOTER_BUILDER_RENDERER ? `?renderer=${process.env.SHOOTER_BUILDER_RENDERER}` : "";
  await page.goto(`http://127.0.0.1:${port}/${renderer}`, { waitUntil: "load" });
  const adapter = await page.evaluate(async () => {
    const info = (await navigator.gpu?.requestAdapter())?.info;
    return info ? `${info.vendor} ${info.architecture} ${info.description}`.trim() : "no WebGPU";
  });
  console.log(`adapter: ${adapter}`);
  let state = await until((s) => s.sockets.length > 0, "the builder opens before the match");
  check(await page.evaluate(() => document.body.hasAttribute("data-builder")), "the page gives the builder the screen");
  const start = state.build;
  const startParts = state.parts;
  // The canvas takes the keys.
  await page.evaluate(() => document.querySelector("canvas")?.focus());
  await shoot("open");

  // 1. A part from the catalogue in hand: the energy rifle (20 × 20 × 40 cm, its length up).
  const rifle = state.targets.find((item) => item.name.includes("rifle"));
  check(rifle, "the catalogue lists the energy rifle");
  await click(rifle.at);
  state = await until((s) => s.hand === rifle.name, "a click on the catalogue takes the rifle in hand");
  check(same(state.turn, [0, 0, 0]), "in hand unturned", state);

  // 2. A socket under the pointer: a fitting socket of the chassis' back (lit, seen).
  state = await settle();
  const free = state.sockets.filter((s) => s.part === 0 && s.fits);
  check(free.length > 0, "the root chassis has fitting sockets in view", state);
  // The socket nearest the middle of the screen among them.
  const middle = [640, 360];
  free.sort((a, b) => Math.hypot(a.at[0] - middle[0], a.at[1] - middle[1]) - Math.hypot(b.at[0] - middle[0], b.at[1] - middle[1]));
  const socket = free[0];
  await hover(socket.at);
  state = await until(
    (s) => s.proposal && s.proposal.parent === 0 && s.proposal.socket === socket.socket,
    `the pointer picks socket ${socket.socket} of the chassis`,
  );
  check(state.proposal.refused === null, "the preview there is valid", state);
  await shoot("picked");

  // 3. Turned in hand about each axis: T pitches a quarter (the preview lies on the face),
  // F rolls a quarter, R yaws a quarter; G steps to an eighth, then a degree.
  await page.keyboard.press("KeyT");
  state = await until((s) => same(s.turn, [0, QUARTER, 0]), "T pitches the part in hand a quarter");
  check(same(state.proposal?.turn, [0, QUARTER, 0]), "the preview is pitched as in hand", state);
  check(state.proposal.footprint.length >= 1, "the pitched preview reserves the cells it lies over", state);
  await shoot("pitched-in-hand");
  await page.keyboard.press("Shift+KeyT");
  state = await until((s) => same(s.turn, [0, 0, 0]), "Shift+T pitches back");
  await page.keyboard.press("KeyF");
  state = await until((s) => same(s.turn, [0, 0, QUARTER]), "F rolls the part in hand a quarter");
  await shoot("rolled-in-hand");
  await page.keyboard.press("Shift+KeyF");
  state = await until((s) => same(s.turn, [0, 0, 0]), "Shift+F rolls back");
  await page.keyboard.press("KeyG");
  state = await until((s) => s.step === 45, "G steps to an eighth");
  await page.keyboard.press("KeyR");
  state = await until((s) => same(s.turn, [EIGHTH, 0, 0]), "R yaws the part in hand an eighth");
  await page.keyboard.press("KeyG");
  state = await until((s) => s.step === 1, "G steps to about a degree");
  await page.keyboard.press("KeyT");
  state = await until((s) => s.turn[0] === EIGHTH && s.turn[1] > 0 && s.turn[1] < 400, "T pitches it a degree");
  await page.keyboard.press("KeyG");
  state = await until((s) => s.step === 90, "G steps back to a quarter");
  // Escape drops the turned part; taken again from the catalogue it comes unturned.
  await page.keyboard.press("Escape");
  await until((s) => s.hand === null, "Escape drops the part in hand");
  await click(rifle.at);
  state = await until((s) => s.hand === rifle.name, "the rifle in hand again");
  check(same(state.turn, [0, 0, 0]), "taken again it is unturned", state);
  await page.keyboard.press("KeyT");
  await hover(socket.at);
  state = await until(
    (s) => same(s.turn, [0, QUARTER, 0]) && s.proposal?.socket === socket.socket && s.proposal.refused === null,
    "pitched a quarter over the socket again, valid",
  );

  // 4. Placed: the build the preview showed, one part more, selected.
  const preview = state.proposal;
  await page.mouse.down();
  await page.mouse.up();
  state = await until((s) => s.parts === startParts + 1, "a click places the pitched rifle");
  const placed = state.build;
  check(state.selected !== null && same(state.selectedTurn, preview.turn), "the placed part is selected, turned as previewed", state);
  check(state.valid, "the build stays valid", state);
  await page.keyboard.press("Escape");
  state = await until((s) => s.hand === null, "Escape drops the part in hand");
  // Off the robot, so the pointer previews nothing.
  await hover([640, 60]);
  await shoot("placed-pitched");

  // 5. Turned again on its socket: R yaws the selected part a quarter (one edit).
  await page.keyboard.press("KeyR");
  state = await until((s) => s.build !== placed, "R turns the selected part on its socket");
  check(same(state.selectedTurn, [QUARTER, QUARTER, 0]), "yawed a quarter on top of its pitch", state);
  const turned = state.build;
  await shoot("turned-on-socket");

  // 6. Undo and redo restore the exact builds.
  await page.keyboard.press("KeyZ");
  state = await until((s) => s.build === placed, "Z undoes the turn: the placed build, exactly");
  await page.keyboard.press("KeyZ");
  state = await until((s) => s.build === start, "Z undoes the placement: the starting build, exactly");
  check(state.parts === startParts, "the part is gone", state);
  await shoot("undone");
  await page.keyboard.press("KeyY");
  state = await until((s) => s.build === placed, "Y redoes the placement, exactly");
  await page.keyboard.press("Shift+KeyZ");
  state = await until((s) => s.build === turned, "Shift+Z redoes the turn, exactly");
  check(!state.redo, "nothing left to redo", state);

  // 7. Refused: the battery (two cells long) in hand over a free socket whose neighbour another
  // part holds is red with its reason, and a click there changes nothing.
  const battery = state.targets.find((item) => item.name === "robot.battery");
  check(battery, "the catalogue lists the battery");
  await click(battery.at);
  await until((s) => s.hand === "robot.battery", "a click takes the battery in hand");
  await hover([640, 60]);
  state = await settle();
  const blocked = state.sockets.find((s) => s.part === 0 && !s.fits);
  check(blocked, "a seen socket of the chassis the battery does not fit", state);
  await hover(blocked.at);
  state = await until(
    (s) => s.proposal && s.proposal.parent === 0 && s.proposal.socket === blocked.socket,
    `the pointer picks socket ${blocked.socket}`,
  );
  check(typeof state.proposal.refused === "string" && state.proposal.refused.length > 0, `refused: ${state.proposal.refused}`, state);
  await shoot("refused");
  await page.mouse.down();
  await page.mouse.up();
  state = await settle();
  check(state.build === turned, "the refused click leaves the build as it was", state);
  check(state.hand === "robot.battery", "the refused part stays in hand", state);

  // 8. Cancelled: Escape drops the hand; a drag from the catalogue cancelled with Escape places
  // nothing.
  await page.keyboard.press("Escape");
  await until((s) => s.hand === null, "Escape drops the refused part");
  await hover(battery.at);
  await page.mouse.down();
  state = await until((s) => s.hand === "robot.battery", "a drag takes the battery");
  const fitting = state.sockets.find((s) => s.part === 0 && s.fits);
  await hover(fitting?.at ?? socket.at);
  await settle();
  await shoot("dragging");
  await page.keyboard.press("Escape");
  await page.mouse.up();
  state = await settle();
  check(state.build === turned && state.hand === null, "Escape cancels the drag: nothing placed, nothing in hand", state);

  // 9. Ready: the match starts with the build on the bench.
  check(state.valid, "the build is valid for Ready", state);
  await click(target(state, "Ready").at);
  await page.waitForFunction(() => !document.body.hasAttribute("data-builder"), null, { timeout: 15_000 });
  check(true, "Ready closes the builder and the match starts");
  await page.waitForTimeout(1_500);
  await shoot("match");
  console.log(`done: ${shots} screenshots in ${out} (${adapter})`);
} catch (error) {
  console.error(`FAILED: ${error.message}\n--- page log (last 30) ---\n${logs.slice(-30).join("\n")}`);
  await shoot("failed").catch(() => {});
  process.exitCode = 1;
} finally {
  await browser.close();
  server.close();
  server.closeAllConnections();
}
