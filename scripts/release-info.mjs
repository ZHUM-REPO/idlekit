// Tells the release workflow what it is about to build, as key=value lines
// (the workflow appends them to $GITHUB_OUTPUT):
//
//   version   the app version, from src-tauri/Cargo.toml
//   tag       the release tag for it
//   exists    true when that release is already published
//   signed    true when the update-signing key is available
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { readVersion, tagFor } from "./lib/release.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = readVersion(fs.readFileSync(path.join(root, "src-tauri", "Cargo.toml"), "utf8"));
const tag = tagFor(version);

// `gh release view` exits with an error when there is no such release.
const repo = process.env.GITHUB_REPOSITORY;
const view = spawnSync("gh", ["release", "view", tag, ...(repo ? ["--repo", repo] : []), "--json", "tagName"], {
  encoding: "utf8",
});
const exists = view.status === 0;
const signed = !!process.env.TAURI_SIGNING_PRIVATE_KEY?.trim();

console.log(`version=${version}`);
console.log(`tag=${tag}`);
console.log(`exists=${exists}`);
console.log(`signed=${signed}`);
