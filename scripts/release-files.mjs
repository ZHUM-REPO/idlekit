// After the build: gathers what gets attached to a release into one folder.
//
//   release/IdleKit_<version>_x64-setup.exe        the installer people download
//   release/IdleKit_<version>_x64-setup.exe.sig    its signature
//   release/latest.json                            what installed copies read to find updates
//   release-notes.md                               the text of the release page
//
// A build made without the signing key has no signature. It is still released,
// as an installer only, and its release page says it cannot update itself.
//
//   node scripts/release-files.mjs [--bundle <folder with the installer>] [--out <folder>]
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { changelogSection, findInstaller, readVersion, releaseNotes, updateManifest } from "./lib/release.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const arg = (name, fallback) => {
  const i = process.argv.indexOf(name);
  return i >= 0 ? path.resolve(process.argv[i + 1]) : fallback;
};
const bundleDir = arg("--bundle", path.join(root, "src-tauri", "target", "release", "bundle", "nsis"));
const outDir = arg("--out", path.join(root, "release"));

const repo = process.env.GITHUB_REPOSITORY;
if (!repo) throw new Error("GITHUB_REPOSITORY (owner/name) is not set");
const version = readVersion(fs.readFileSync(path.join(root, "src-tauri", "Cargo.toml"), "utf8"));
const installer = findInstaller(fs.readdirSync(bundleDir), version);
const signatureFile = path.join(bundleDir, `${installer}.sig`);
const signed = fs.existsSync(signatureFile);
const changelogFile = path.join(root, "CHANGELOG.md");
const changes = fs.existsSync(changelogFile) ? changelogSection(fs.readFileSync(changelogFile, "utf8"), version) : "";

fs.rmSync(outDir, { recursive: true, force: true });
fs.mkdirSync(outDir, { recursive: true });
fs.copyFileSync(path.join(bundleDir, installer), path.join(outDir, installer));
if (signed) {
  fs.copyFileSync(signatureFile, path.join(outDir, `${installer}.sig`));
  const manifest = updateManifest({ version, repo, installer, signature: fs.readFileSync(signatureFile, "utf8"), notes: changes });
  fs.writeFileSync(path.join(outDir, "latest.json"), JSON.stringify(manifest, null, 2) + "\n");
} else {
  // Shown as a warning on the workflow run.
  console.log(`::warning::${installer} has no signature, so this release cannot be installed as an update. Check the TAURI_SIGNING_PRIVATE_KEY secret.`);
}
fs.writeFileSync(path.join(root, "release-notes.md"), releaseNotes({ version, installer, changes, selfUpdating: signed }));

for (const name of fs.readdirSync(outDir).sort()) {
  console.log(`${path.relative(root, path.join(outDir, name))}  (${fs.statSync(path.join(outDir, name)).size} bytes)`);
}
