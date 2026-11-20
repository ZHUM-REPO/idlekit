// Builds the steam-helper sidecar and puts it where Tauri expects it
// (src-tauri/binaries/steam-helper-<target-triple>[.exe]), and copies Valve's
// Steam API redistributable (shipped inside the steamworks-sys crate) into
// src-tauri/resources/ for bundling and next to the dev binaries.
//
// Usage: node scripts/prepare-helper.mjs [--release] [--target <triple>]
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const tauriDir = path.join(root, "src-tauri");
const args = process.argv.slice(2);
const release = args.includes("--release");
const targetIdx = args.indexOf("--target");
const explicitTarget = targetIdx >= 0 ? args[targetIdx + 1] : process.env.TAURI_ENV_TARGET_TRIPLE;

function hostTriple() {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const line = out.split("\n").find((l) => l.startsWith("host:"));
  if (!line) throw new Error("could not determine the Rust host target");
  return line.slice(5).trim();
}

const triple = explicitTarget || hostTriple();
const isWindows = triple.includes("windows");
const exe = isWindows ? ".exe" : "";
const profile = release ? "release" : "debug";

const cargoArgs = ["build", "-p", "steam-helper"];
if (release) cargoArgs.push("--release");
if (explicitTarget) cargoArgs.push("--target", explicitTarget);
console.log(`> cargo ${cargoArgs.join(" ")}`);
execFileSync("cargo", cargoArgs, { cwd: tauriDir, stdio: "inherit" });

const targetDir = path.join(tauriDir, "target", ...(explicitTarget ? [explicitTarget] : []), profile);
const builtHelper = path.join(targetDir, `steam-helper${exe}`);
if (!fs.existsSync(builtHelper)) throw new Error(`helper not found at ${builtHelper}`);

const binDir = path.join(tauriDir, "binaries");
fs.mkdirSync(binDir, { recursive: true });
const sidecar = path.join(binDir, `steam-helper-${triple}${exe}`);
fs.copyFileSync(builtHelper, sidecar);
if (!isWindows) fs.chmodSync(sidecar, 0o755);
console.log(`  helper  -> ${path.relative(root, sidecar)}`);

const libName = isWindows
  ? triple.startsWith("i686")
    ? "steam_api.dll"
    : "steam_api64.dll"
  : triple.includes("apple")
    ? "libsteam_api.dylib"
    : "libsteam_api.so";

// steamworks-sys copies the redistributable into its build OUT_DIR.
const buildDir = path.join(targetDir, "build");
let lib = null;
for (const d of fs.existsSync(buildDir) ? fs.readdirSync(buildDir) : []) {
  if (!d.startsWith("steamworks-sys-")) continue;
  const candidate = path.join(buildDir, d, "out", libName);
  if (fs.existsSync(candidate)) {
    lib = candidate;
    break;
  }
}
if (!lib) throw new Error(`${libName} not found under ${buildDir}`);

const resDir = path.join(tauriDir, "resources");
fs.mkdirSync(resDir, { recursive: true });
fs.copyFileSync(lib, path.join(resDir, libName));
fs.copyFileSync(lib, path.join(targetDir, libName));
console.log(`  ${libName} -> src-tauri/resources/ and target/${profile}/`);
