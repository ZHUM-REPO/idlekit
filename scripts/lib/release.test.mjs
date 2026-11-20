import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import {
  buildOverride,
  changelogSection,
  findInstaller,
  readVersion,
  releaseNotes,
  updateEndpoint,
  updateManifest,
} from "./release.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

test("reads the version from the [package] section only", () => {
  const toml = '[workspace]\nmembers = ["helper"]\n\n[package]\nname = "idlekit"\nversion = "1.4.2"\n\n[dependencies]\nfoo = { version = "9.9.9" }\n';
  assert.equal(readVersion(toml), "1.4.2");
  assert.throws(() => readVersion('[dependencies]\nfoo = "1"\n'), /no `version/);
  assert.throws(() => readVersion('[package]\nversion = "next"\n'), /not a version/);
});

test("the version is set in Cargo.toml and nowhere else", () => {
  // Raising one number in one file has to be enough to make a release.
  readVersion(fs.readFileSync(path.join(root, "src-tauri/Cargo.toml"), "utf8"));
  const conf = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
  assert.equal(conf.version, undefined, "tauri.conf.json should leave the version to Cargo.toml");
});

test("points the updater at the repository's latest release", () => {
  assert.equal(updateEndpoint("someone/idlekit"), "https://github.com/someone/idlekit/releases/latest/download/latest.json");
  assert.throws(() => updateEndpoint("not a repo"));
  assert.throws(() => updateEndpoint(undefined));
});

test("an unsigned build has nowhere to look for updates", () => {
  assert.deepEqual(buildOverride({ repo: "someone/idlekit", signed: true }), {
    bundle: { createUpdaterArtifacts: true },
    plugins: { updater: { endpoints: ["https://github.com/someone/idlekit/releases/latest/download/latest.json"] } },
  });
  assert.deepEqual(buildOverride({ repo: "someone/idlekit", signed: false }), {
    bundle: { createUpdaterArtifacts: false },
    plugins: { updater: { endpoints: [] } },
  });
});

test("takes one version's section from the changelog", () => {
  const log = "# Changelog\n\n## 0.3.0\n\n- New thing\n- Other thing\n\n## 0.2.0\n\n- Older\n";
  assert.equal(changelogSection(log, "0.3.0"), "- New thing\n- Other thing");
  assert.equal(changelogSection(log, "0.2.0"), "- Older");
  assert.equal(changelogSection(log, "0.2"), "");
  assert.equal(changelogSection(log, "9.9.9"), "");
  assert.equal(changelogSection("## v1.0.0 (2026-11-20)\r\n\r\n- Dated heading\r\n", "1.0.0"), "- Dated heading");
});

test("finds the installer for this version", () => {
  const files = ["IdleKit_0.3.0_x64-setup.exe", "IdleKit_0.3.0_x64-setup.exe.sig", "IdleKit_0.2.0_x64-setup.exe"];
  assert.equal(findInstaller(files, "0.3.0"), "IdleKit_0.3.0_x64-setup.exe");
  assert.throws(() => findInstaller(files, "0.4.0"), /found 0/);
  assert.throws(() => findInstaller([], "0.3.0"), /no installers/);
});

test("writes a manifest the updater can read", () => {
  const manifest = updateManifest({
    version: "0.3.0",
    repo: "someone/idlekit",
    installer: "IdleKit_0.3.0_x64-setup.exe",
    signature: "c2lnbmF0dXJl\n",
    notes: "- New thing",
    date: new Date("2026-11-20T12:00:00Z"),
  });
  assert.equal(manifest.version, "0.3.0");
  assert.equal(manifest.pub_date, "2026-11-20T12:00:00.000Z");
  const entry = manifest.platforms["windows-x86_64"];
  assert.equal(entry.signature, "c2lnbmF0dXJl");
  assert.equal(entry.url, "https://github.com/someone/idlekit/releases/download/v0.3.0/IdleKit_0.3.0_x64-setup.exe");
  assert.deepEqual(manifest.platforms["windows-x86_64-nsis"], entry);
  assert.throws(() => updateManifest({ version: "0.3.0", repo: "a/b", installer: "x.exe", signature: " " }), /no signature/);
});

test("release notes name the file to download", () => {
  const text = releaseNotes({ version: "0.3.0", installer: "IdleKit_0.3.0_x64-setup.exe", changes: "- New thing" });
  assert.match(text, /^- New thing/);
  assert.match(text, /\*\*IdleKit_0\.3\.0_x64-setup\.exe\*\*/);
  assert.match(text, /offers this version itself/);
  assert.match(releaseNotes({ version: "0.3.0", installer: "a.exe", changes: "" }), /^IdleKit 0\.3\.0/);
  assert.match(releaseNotes({ version: "0.3.0", installer: "a.exe", selfUpdating: false }), /cannot update itself/);
});
