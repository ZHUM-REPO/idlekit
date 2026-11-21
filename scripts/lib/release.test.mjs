import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import {
  buildOverride,
  changelogSection,
  checkNumbering,
  findInstaller,
  nextVersion,
  readVersion,
  releaseNotes,
  shownVersion,
  signatureMatchesKey,
  tagFor,
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

test("releases are named with two numbers and numbered one after another", () => {
  assert.equal(shownVersion("0.5.0"), "0.5");
  assert.equal(shownVersion("1.0.0"), "1.0");
  assert.equal(shownVersion("0.2.2"), "0.2.2");
  assert.equal(tagFor("0.5.0"), "v0.5");
  assert.equal(nextVersion("0.5.0"), "0.6.0");
  assert.equal(nextVersion("0.9.0"), "1.0.0");
  assert.equal(nextVersion("1.9.0"), "2.0.0");
  checkNumbering("0.5.0");
  checkNumbering("0.9.0");
  checkNumbering("1.0.0");
  assert.throws(() => checkNumbering("0.5.1"), /last number stays 0.*next one is 0\.6\.0/);
  assert.throws(() => checkNumbering("0.10.0"), /after 0\.9\.0 comes 1\.0\.0/);
  // the version this repository is at follows the rule
  checkNumbering(readVersion(fs.readFileSync(path.join(root, "src-tauri/Cargo.toml"), "utf8")));
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
  // a heading may name the release the short way
  assert.equal(changelogSection("## 0.5\n\n- Short name\n\n## 0.2.2\n\n- Before\n", "0.5.0"), "- Short name");
  assert.equal(changelogSection("## 0.5\n\n- Short name\n", "0.5.1"), "");
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
  assert.deepEqual(manifest.platforms["windows-x86_64-nsis"], entry);
  assert.equal(entry.url, "https://github.com/someone/idlekit/releases/download/v0.3/IdleKit_0.3.0_x64-setup.exe");
  assert.throws(() => updateManifest({ version: "0.3.0", repo: "a/b", installer: "x.exe", signature: " " }), /no signature/);
});

test("release notes name the file to download", () => {
  const text = releaseNotes({ version: "0.3.0", installer: "IdleKit_0.3.0_x64-setup.exe", changes: "- New thing" });
  assert.match(text, /^- New thing/);
  assert.match(text, /\*\*IdleKit_0\.3\.0_x64-setup\.exe\*\*/);
  assert.match(text, /offers this version itself/);
  assert.match(releaseNotes({ version: "0.3.0", installer: "a.exe", changes: "" }), /^IdleKit 0\.3\n/);
  assert.match(releaseNotes({ version: "0.3.0", installer: "a.exe", selfUpdating: false }), /cannot update itself/);
});

// An update key and a signature in the formats `tauri signer` writes them.
function signingKey() {
  const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
  const id = crypto.randomBytes(8);
  const raw = publicKey.export({ format: "der", type: "spki" }).subarray(-32);
  const b64 = (buffer) => Buffer.from(buffer).toString("base64");
  const pubkey = b64(`untrusted comment: minisign public key\n${b64(Buffer.concat([Buffer.from("Ed"), id, raw]))}\n`);
  const sign = (file) => {
    const signature = crypto.sign(null, crypto.createHash("blake2b512").update(file).digest(), privateKey);
    const trusted = "timestamp:1\tfile:setup.exe";
    const global = crypto.sign(null, Buffer.concat([signature, Buffer.from(trusted)]), privateKey);
    const lines = ["untrusted comment: signature from tauri secret key", b64(Buffer.concat([Buffer.from("ED"), id, signature])), `trusted comment: ${trusted}`, b64(global)];
    return b64(lines.join("\n") + "\n");
  };
  return { pubkey, sign };
}

test("a signature counts only for the same bytes and the app's own key", () => {
  const ours = signingKey();
  const theirs = signingKey();
  const file = Buffer.from("the installer");
  assert.equal(signatureMatchesKey({ pubkey: ours.pubkey, file, signature: ours.sign(file) }), true);
  assert.equal(signatureMatchesKey({ pubkey: ours.pubkey, file: Buffer.from("the installer!"), signature: ours.sign(file) }), false);
  assert.equal(signatureMatchesKey({ pubkey: ours.pubkey, file, signature: theirs.sign(file) }), false);
  assert.equal(signatureMatchesKey({ pubkey: ours.pubkey, file, signature: "not a signature" }), false);
  assert.equal(signatureMatchesKey({ pubkey: "", file, signature: ours.sign(file) }), false);
});
