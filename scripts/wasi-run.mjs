// Runs a wasm32-wasip1 program under Node's WASI: `node scripts/wasi-run.mjs PROGRAM.wasm [ARGS…]`.
// Also a Cargo runner (CARGO_TARGET_WASM32_WASIP1_RUNNER) for shooter-sim's wasm32 builds.
import { readFile } from "node:fs/promises";
import { WASI } from "node:wasi";
import process from "node:process";

const [program, ...args] = process.argv.slice(2);
const wasi = new WASI({ version: "preview1", args: [program, ...args], env: {}, returnOnExit: true });
const module = await WebAssembly.compile(await readFile(program));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
process.exitCode = wasi.start(instance);
