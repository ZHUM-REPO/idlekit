// The parts of releasing that are plain data handling, kept apart from the
// scripts that run them so they can be tested (npm run test:scripts).

/** The app version: `version = "…"` under [package] in src-tauri/Cargo.toml. */
export function readVersion(cargoToml) {
  const section = cargoToml.split(/^\[/m).find((s) => s.startsWith("package]"));
  const match = section && /^version\s*=\s*"([^"]+)"/m.exec(section);
  if (!match) throw new Error('no `version = "…"` found under [package] in src-tauri/Cargo.toml');
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(match[1])) {
    throw new Error(`"${match[1]}" is not a version like 1.2.3`);
  }
  return match[1];
}

export const tagFor = (version) => `v${version}`;

/** Where installed copies look for updates: a file attached to the repository's latest release. */
export function updateEndpoint(repo) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo ?? "")) throw new Error(`"${repo}" is not an owner/name repository`);
  return `https://github.com/${repo}/releases/latest/download/latest.json`;
}

/** What `tauri build` is given on top of tauri.conf.json for a release build. */
export function buildOverride({ repo, signed }) {
  return {
    bundle: { createUpdaterArtifacts: !!signed },
    // Without a signature an installed copy would refuse every update, so an
    // unsigned build is told there is nowhere to look.
    plugins: { updater: { endpoints: signed ? [updateEndpoint(repo)] : [] } },
  };
}

/** The text under `## <version>` in CHANGELOG.md, or "" when there is none. */
export function changelogSection(changelog, version) {
  const lines = changelog.split(/\r?\n/);
  // The version has to be the whole thing: "0.2" must not match "## 0.2.0".
  const heading = new RegExp(`^##\\s+v?${version.replace(/\./g, "\\.")}(?![\\w.-])`);
  const start = lines.findIndex((l) => heading.test(l));
  if (start < 0) return "";
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((l) => /^##\s/.test(l));
  return (end < 0 ? rest : rest.slice(0, end)).join("\n").trim();
}

/** The installer among the files `tauri build` left in bundle/nsis. */
export function findInstaller(fileNames, version) {
  const installers = fileNames.filter((n) => n.endsWith("-setup.exe"));
  const mine = installers.filter((n) => n.includes(`_${version}_`));
  if (mine.length !== 1) {
    throw new Error(
      `expected one installer for ${version} in the bundle folder, found ${mine.length} (${installers.join(", ") || "no installers"})`,
    );
  }
  return mine[0];
}

/** latest.json: what an installed copy downloads to learn about a newer version. */
export function updateManifest({ version, repo, installer, signature, notes, date }) {
  if (!signature?.trim()) throw new Error("the installer has no signature");
  const entry = {
    signature: signature.trim(),
    url: `https://github.com/${repo}/releases/download/${tagFor(version)}/${encodeURIComponent(installer)}`,
  };
  return {
    version,
    notes: notes || `IdleKit ${version}`,
    pub_date: (date ?? new Date()).toISOString(),
    platforms: { "windows-x86_64": entry, "windows-x86_64-nsis": entry },
  };
}

/** The text of the release page. */
export function releaseNotes({ version, installer, changes, selfUpdating = true }) {
  return [
    changes || `IdleKit ${version}`,
    "",
    "## Installing",
    "",
    `Download **${installer}** below and run it. Windows may say the publisher is unknown, because the installer isn't code-signed: choose **More info**, then **Run anyway**.`,
    "",
    selfUpdating
      ? "If IdleKit is already installed it offers this version itself; there is no need to download anything."
      : "This build was made without the update-signing key, so it cannot update itself. Later versions have to be downloaded and installed by hand until one is built with the key.",
    "",
  ].join("\n");
}
