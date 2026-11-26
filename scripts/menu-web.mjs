// The main menu clicked through in a real browser (shooter#154): serves web/dist on loopback,
// opens the page with no query (the main menu) in headless Chromium at 1280x720 on WebGPU and
// drives it with the mouse as a person would: Practice on each of its three arenas, a fighter
// count chosen and the match started, Leave back to the menu, Play Online's bot match and back,
// the Robot Workshop and its Back, the Workshop's active build (shooter#155: a loadout saved as
// SCOUT, made active with Done, an edit dropped with Back, still active after a reload and played
// by a practice match), and Settings with the developer connection. Every step checks
// what the page's readout (`window.shooterMenu`: the menu's page, its choice and where its items
// are, or the match's map, fighters and label) says against what the step should do, and
// screenshots the page into OUT. Nothing connects anywhere: the page makes no network request
// beyond its own files.
//
// Usage (after scripts/build-web.sh): node scripts/menu-web.mjs OUT
// e.g. node scripts/menu-web.mjs docs/captures/154
// It exits non-zero on the first step that does not do what it should.
//
// WebGPU runs on the machine's GPU (Vulkan), as scripts/capture-web.mjs; `?renderer=lite` (the
// WebGL 2 renderer) by SHOOTER_MENU_RENDERER=lite.
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
  console.error("usage: node scripts/menu-web.mjs OUT");
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
const { port } = server.address();
const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
const logs = [];
page.on("console", (message) => logs.push(message.text()));
page.on("pageerror", (error) => logs.push(`page error: ${error.message}`));
// Every request stays on the page's own server: no host, no connection attempt.
const foreign = [];
page.on("request", (request) => {
  if (!request.url().startsWith(`http://127.0.0.1:${port}/`)) foreign.push(request.url());
});

let shots = 0;
function check(condition, what, state) {
  if (!condition) {
    throw new Error(`${what}\n${state ? JSON.stringify({ ...state, targets: undefined }) : ""}`);
  }
  console.log(`ok: ${what}`);
}

const read = () => page.evaluate(() => Reflect.get(window, "shooterMenu") ?? null);
const builder = () => page.evaluate(() => Reflect.get(window, "shooterBuilder") ?? null);

/** The readout once `ready(state)` holds. */
async function until(ready, what, source = read) {
  const deadline = Date.now() + 20_000;
  let state = await source();
  while (!(state && ready(state))) {
    if (Date.now() > deadline) check(false, `timed out: ${what}`, state);
    await page.waitForTimeout(50);
    state = await source();
  }
  check(true, what);
  return state;
}

async function shoot(name) {
  shots += 1;
  const file = path.join(out, `browser-${String(shots).padStart(2, "0")}-${name}.png`);
  await page.screenshot({ path: file });
  console.log(`   ${file}`);
}

/** Clicks the readout's item called `name` (its position is in the window's device pixels). */
async function click(state, name) {
  const target = state.targets.find((item) => item.name === name);
  check(target, `${name} is on the ${state.screen} page`, state);
  const ratio = await page.evaluate(() => devicePixelRatio);
  const [x, y] = target.at.map((value) => value / ratio);
  await page.mouse.move(x, y, { steps: 3 });
  await page.mouse.down();
  await page.mouse.up();
}

const settle = (frames = 30) =>
  page.evaluate(
    (count) =>
      new Promise((done) => {
        const step = (left) => (left <= 0 ? done() : requestAnimationFrame(() => step(left - 1)));
        step(count);
      }),
    frames,
  );

try {
  const renderer = process.env.SHOOTER_MENU_RENDERER ? `?renderer=${process.env.SHOOTER_MENU_RENDERER}` : "";
  await page.goto(`http://127.0.0.1:${port}/${renderer}`, { waitUntil: "load" });
  const adapter = await page.evaluate(async () => {
    const info = (await navigator.gpu?.requestAdapter())?.info;
    return info ? `${info.vendor} ${info.architecture} ${info.description}`.trim() : "no WebGPU";
  });
  console.log(`adapter: ${adapter}`);

  // 1. The main menu, nothing playing, the page's own overlay hidden.
  let state = await until((s) => s.screen === "Main", "the page opens on the main menu");
  check(
    await page.evaluate(() => document.body.hasAttribute("data-menu")),
    "the page marks the menu (its click-to-play overlay hidden)",
  );
  // The arena and robot kits in, the live view drawn.
  await page.waitForTimeout(2_500);
  await settle();
  await shoot("menu");

  // 2. Practice: each arena chosen, the live view following.
  await click(state, "Practice");
  state = await until((s) => s.screen === "Practice", "Practice opens");
  for (const map of ["proving-cell", "assembly-works", "drydock"]) {
    await click(state, `Map:${map}`);
    state = await until((s) => s.map === map, `Practice has ${map} chosen`);
    await page.waitForTimeout(1_000);
    await settle();
    await shoot(`practice-${map}`);
  }
  check(
    JSON.stringify(state.range) === "[8,12]" && state.fighters === 12,
    "the drydock offers 8 to 12 fighters, 12 by default",
    state,
  );

  // 3. The proving cell with three fighters, started: the match plays it.
  await click(state, "Map:proving-cell");
  state = await until((s) => s.map === "proving-cell" && s.fighters === 6, "the proving cell starts at 6");
  for (const fewer of [5, 4, 3]) {
    await click(state, "Fewer");
    state = await until((s) => s.fighters === fewer, `${fewer} fighters`);
  }
  await click(state, "Start");
  state = await until(
    (s) => s.screen === "Match" && s.map === "proving-cell" && s.fighters === 3 && s.inProcess,
    "Start plays the proving cell with three fighters, in the page",
  );
  check(
    !(await page.evaluate(() => document.body.hasAttribute("data-menu"))),
    "the page shows the match (its click-to-play overlay back)",
  );
  await page.waitForTimeout(2_000);
  await shoot("practice-match");

  // 4. Leave (the pointer is free): back on the menu.
  state = await until((s) => s.targets.some((t) => t.name === "Leave"), "the match offers Leave");
  await click(state, "Leave");
  state = await until((s) => s.screen === "Main", "Leave returns to the main menu");

  // 5. Play Online: at once a 12-fighter bot match on the assembly works, labelled offline.
  await click(state, "PlayOnline");
  state = await until(
    (s) => s.screen === "Match" && s.map === "assembly-works" && s.fighters === 12 && s.inProcess,
    "Play Online is an in-page 12-fighter match on the assembly works",
  );
  check(/offline/i.test(state.label ?? ""), "it says it is an offline bot match", state);
  check(
    /offline/i.test(await page.evaluate(() => document.getElementById("mode")?.textContent ?? "")),
    "the page's click-to-play line says so too",
  );
  await page.waitForTimeout(2_000);
  await shoot("play-online");
  await click(state, "Leave");
  state = await until((s) => s.screen === "Main", "Leave returns to the main menu again");

  // 6. The Robot Workshop: the builder with no match, and Back.
  await click(state, "Workshop");
  let bench = await until((s) => s.sockets?.length > 0, "the Workshop opens the builder", builder);
  state = await until((s) => s.screen === "Workshop" && s.map === null, "no match plays in the Workshop");
  await settle();
  await shoot("workshop");
  {
    const back = bench.targets.find((item) => item.name === "Back");
    check(back, "the Workshop has Back", bench);
    const ratio = await page.evaluate(() => devicePixelRatio);
    await page.mouse.move(back.at[0] / ratio, back.at[1] / ratio, { steps: 3 });
    await page.mouse.down();
    await page.mouse.up();
  }
  state = await until((s) => s.screen === "Main", "Back from the Workshop is the menu");

  // 6b. The Workshop's active build (shooter#155): the marksman loadout saved as SCOUT and made
  // active with Done, an edit dropped with Back, and after a reload (a restart) the menu and a
  // practice match still play it.
  const clickBench = async (bench, name) => {
    const target = bench.targets.find((item) => item.name === name);
    check(target, `${name} is on the builder`, bench);
    const ratio = await page.evaluate(() => devicePixelRatio);
    await page.mouse.move(target.at[0] / ratio, target.at[1] / ratio, { steps: 3 });
    await page.mouse.down();
    await page.mouse.up();
  };
  await click(state, "Workshop");
  bench = await until((s) => s.sockets?.length > 0 && !s.ready, "the Workshop opens again", builder);
  await clickBench(bench, "Loadout(3)");
  bench = await until((s) => s.parts > 0 && s.valid && s.build !== bench.build, "the marksman loadout is on the bench", builder);
  const scout = bench.build;
  await page.evaluate(() => document.querySelector("canvas")?.focus());
  await page.keyboard.press("KeyB");
  for (const key of ["KeyS", "KeyC", "KeyO", "KeyU", "KeyT", "Enter"]) await page.keyboard.press(key);
  bench = await until((s) => s.notice === "saved SCOUT", "it is saved as SCOUT", builder);
  await settle();
  await shoot("workshop-saved");
  await page.keyboard.press("Escape");
  bench = await builder();
  await clickBench(bench, "Ready");
  state = await until(
    (s) => s.screen === "Main" && /^Your robot: SCOUT/.test(s.robot ?? ""),
    "Done makes SCOUT the active build and returns to the menu",
  );
  check(
    (await page.evaluate(() => localStorage.getItem("shooter.active")))?.startsWith("53484244"),
    "the active build is kept in the page's storage",
  );
  await click(state, "Workshop");
  bench = await until((s) => s.sockets?.length > 0 && s.build === scout, "the Workshop opens on SCOUT", builder);
  await clickBench(bench, "Loadout(1)");
  bench = await until((s) => s.build !== scout, "another loadout is on the bench", builder);
  await clickBench(bench, "Back");
  state = await until(
    (s) => s.screen === "Main" && /^Your robot: SCOUT/.test(s.robot ?? ""),
    "Back drops the edit, SCOUT stays active",
  );
  await page.reload({ waitUntil: "load" });
  state = await until(
    (s) => s.screen === "Main" && /^Your robot: SCOUT/.test(s.robot ?? ""),
    "after a reload the menu still plays SCOUT",
  );
  await page.waitForTimeout(2_500);
  await settle();
  await shoot("menu-active-build");
  await click(state, "Practice");
  state = await until((s) => s.screen === "Practice", "Practice opens");
  await click(state, "Map:proving-cell");
  state = await until((s) => s.map === "proving-cell", "the proving cell is chosen");
  await click(state, "Start");
  state = await until(
    (s) => s.screen === "Match" && s.map === "proving-cell" && s.inProcess && s.build === scout,
    "the practice match plays SCOUT, in the page",
  );
  await page.waitForTimeout(2_000);
  await shoot("practice-active-build");
  state = await until((s) => s.targets.some((t) => t.name === "Leave"), "the match offers Leave");
  await click(state, "Leave");
  state = await until((s) => s.screen === "Main", "Leave returns to the main menu");

  // 7. Settings: the developer connection only under Advanced.
  await click(state, "Settings");
  state = await until((s) => s.screen === "Settings", "Settings opens");
  check(!state.targets.some((t) => t.name === "Url"), "no server URL before Advanced", state);
  await click(state, "Faster");
  state = await until((s) => s.sensitivity === 110, "the look sensitivity steps up");
  await click(state, "Advanced");
  state = await until((s) => s.targets.some((t) => t.name === "Url"), "Advanced shows the developer connection");
  await settle();
  await shoot("settings");
  check(
    (await page.evaluate(() => localStorage.getItem("shooter.sensitivity"))) === "110",
    "the sensitivity is kept in the page's storage",
  );

  check(foreign.length === 0, `no request left the page's server (${foreign.join(", ")})`);

  // 8. With SHOOTER_MENU_CONNECT=<match url> and SHOOTER_MENU_CERTHASH=<sha-256> (a host running
  // as scripts/dev.sh starts it): the developer connection joins it, typed into the page's prompts.
  const url = process.env.SHOOTER_MENU_CONNECT;
  if (url) {
    const answers = [url, process.env.SHOOTER_MENU_CERTHASH ?? ""];
    page.on("dialog", (dialog) => dialog.accept(answers.shift() ?? ""));
    await click(state, "Url");
    await click(state, "Certificate");
    await click(state, "Connect");
    state = await until(
      (s) => s.screen === "Match" && !s.inProcess && s.map !== null && s.fighters > 0,
      "the developer connection joins the host (its map and slots arrive)",
    );
    console.log(`   hosted: ${state.map}, ${state.fighters} slots`);
    await page.waitForTimeout(2_000);
    await shoot("hosted-match");
    await click(state, "Leave");
    state = await until((s) => s.screen === "Main", "Leave ends the hosted match");
  }
} catch (error) {
  console.error(String(error.message ?? error));
  console.error(logs.slice(-30).join("\n"));
  await shoot("failed").catch(() => {});
  process.exitCode = 1;
} finally {
  await browser.close();
  server.close();
}
