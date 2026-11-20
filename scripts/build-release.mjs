// Builds the installer the way a release needs it. On top of tauri.conf.json:
//
//   - the updater is pointed at this repository's releases, so an installed
//     copy knows where to look for the next version;
//   - when the signing key is present (TAURI_SIGNING_PRIVATE_KEY), the
//     installer gets a signature, which installed copies check before
//     accepting an update.
//
// Used by .github/workflows/release.yml. Run steam-helper's build first
// (node scripts/prepare-helper.mjs --release).
//
//   node scripts/build-release.mjs [--print]     --print shows the command without running it
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildOverride } from "./lib/release.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// The CLI's own script is run with this Node, so nothing depends on how the
// shell quotes arguments.
const cli = path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
const targetDir = path.join(root, "src-tauri", "target");
fs.mkdirSync(targetDir, { recursive: true });
// An empty password, said explicitly, keeps the CLI from asking for one.
const env = { ...process.env, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "" };

/** Whether the signing key works: a key pasted wrongly should cost the update feature, not the whole build. */
function keyWorks() {
  if (!process.env.TAURI_SIGNING_PRIVATE_KEY?.trim()) return false;
  const probe = path.join(targetDir, "signing-check.txt");
  fs.writeFileSync(probe, "IdleKit signing check\n");
  const result = spawnSync(process.execPath, [cli, "signer", "sign", probe], { cwd: root, env, encoding: "utf8" });
  const ok = result.status === 0 && fs.existsSync(`${probe}.sig`);
  fs.rmSync(probe, { force: true });
  fs.rmSync(`${probe}.sig`, { force: true });
  if (!ok) {
    const why = (result.stderr || result.stdout || "").trim().split(/\r?\n/).pop() || "no reason given";
    console.log(`::warning::The TAURI_SIGNING_PRIVATE_KEY secret could not be used to sign (${why}). Building without it: this build will not be able to update itself.`);
  }
  return ok;
}

const repo = process.env.GITHUB_REPOSITORY;
const signed = !process.argv.includes("--print") ? keyWorks() : !!process.env.TAURI_SIGNING_PRIVATE_KEY?.trim();
if (signed && !repo) throw new Error("GITHUB_REPOSITORY (owner/name) is needed to point the updater at the releases");
if (!signed) delete env.TAURI_SIGNING_PRIVATE_KEY;

const override = buildOverride({ repo, signed });
const overrideFile = path.join(targetDir, "release-config.json");
fs.writeFileSync(overrideFile, JSON.stringify(override, null, 2));

const args = [cli, "build", "--config", overrideFile];
if (process.platform === "win32") args.push("--bundles", "nsis");

console.log(signed ? `Updates will be looked for at ${override.plugins.updater.endpoints[0]}` : "No signing key: this build will not be able to update itself");
console.log(`> tauri ${args.slice(1).map((a) => path.relative(root, a) || a).join(" ")}`);
if (process.argv.includes("--print")) process.exit(0);

const result = spawnSync(process.execPath, args, { cwd: root, stdio: "inherit", env });
process.exit(result.status ?? 1);
