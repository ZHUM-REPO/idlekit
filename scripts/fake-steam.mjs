// A local stand-in for Steam's store API and image hosts, for working on
// IdleKit's game art without reaching Steam.
//
//   node scripts/fake-steam.mjs [port]            (default 4599)
//
// Use it two ways:
//   Browser preview:  npm run dev, then open  http://localhost:1420/?art=http://127.0.0.1:4599
//   Desktop app:      IDLEKIT_STEAM_API=http://127.0.0.1:4599 IDLEKIT_STEAM_CDN=http://127.0.0.1:4599 npm run app:dev
//
// It serves:
//   /IStoreBrowseService/GetItems/v1/?input_json=...     the store lookup
//   /store_item_assets/steam/apps/<id>/[hash/]<file>     banner and cover
//   /community_assets/images/apps/<id>/<file>            icon
// The pictures are generated abstract SVGs marked "sample art", not real game
// art. A few ids are rigged to fail in specific ways so each fallback can be
// seen: no cover (Terraria, Deep Rock Galactic), no art at all (Cuphead), no
// icon (Valheim), unknown to the store (Garry's Mod), hashed folders (Portal 2,
// Disco Elysium; set FAKE_STEAM_STRICT=1 to refuse their plain paths as Steam does).
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";

const port = Number(process.argv[2] || 4599);
const LOG = process.env.FAKE_STEAM_LOG || path.join(os.tmpdir(), "fake-steam.log");
fs.writeFileSync(LOG, "");
const log = (line) => fs.appendFileSync(LOG, line + "\n");

const NAMES = {
  730: "Counter-Strike 2", 570: "Dota 2", 440: "Team Fortress 2", 620: "Portal 2", 1145360: "Hades",
  413150: "Stardew Valley", 105600: "Terraria", 367520: "Hollow Knight", 504230: "Celeste",
  646570: "Slay the Spire", 1794680: "Vampire Survivors", 2379780: "Balatro", 548430: "Deep Rock Galactic",
  632360: "Risk of Rain 2", 427520: "Factorio", 294100: "RimWorld", 588650: "Dead Cells", 268910: "Cuphead",
  753640: "Outer Wilds", 632470: "Disco Elysium", 1086940: "Baldur's Gate 3", 892970: "Valheim",
  1091500: "Cyberpunk 2077", 1966720: "Lethal Company", 1868140: "Dave the Diver", 4000: "Garry's Mod",
};

// Rigged failures.
const NO_COVER = new Set([105600, 548430]); // has a banner but no portrait
const NO_ART = new Set([268910]); // every image 404s
const NO_ICON = new Set([892970]); // icon 404s
const DELISTED = new Set([4000]); // the store API knows nothing about it
const HASHED = new Set([620, 632470]); // art only exists in hashed folders
const hashOf = (id) => (id * 2654435761 % 0xffffffff).toString(16).padStart(8, "0").slice(0, 8);

const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
function rng(seed) {
  let s = seed >>> 0 || 1;
  return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
}

function art(id, w, h) {
  const r = rng(id * 7919);
  const hue = Math.floor(r() * 360);
  const hue2 = (hue + 30 + Math.floor(r() * 60)) % 360;
  const name = NAMES[id] || `App ${id}`;
  let shapes = "";
  for (let i = 0; i < 5; i++) {
    const cx = r() * w, cy = r() * h * 0.8, rad = (0.12 + r() * 0.3) * Math.min(w, h);
    const k = (hue + 150 + i * 23) % 360;
    shapes += i % 2
      ? `<circle cx="${cx.toFixed(0)}" cy="${cy.toFixed(0)}" r="${rad.toFixed(0)}" fill="hsl(${k} 70% 62%)" opacity="0.38"/>`
      : `<rect x="${(cx - rad).toFixed(0)}" y="${(cy - rad / 2).toFixed(0)}" width="${(rad * 2).toFixed(0)}" height="${rad.toFixed(0)}" rx="${(rad / 6).toFixed(0)}" transform="rotate(${(r() * 70 - 35).toFixed(0)} ${cx.toFixed(0)} ${cy.toFixed(0)})" fill="hsl(${k} 75% 70%)" opacity="0.3"/>`;
  }
  const portrait = h > w;
  const title = portrait ? Math.round(w * 0.105) : Math.round(h * 0.2);
  const words = name.split(" ");
  const lines = [];
  for (const word of words) {
    const last = lines[lines.length - 1];
    if (last && (last + " " + word).length <= (portrait ? 11 : 20)) lines[lines.length - 1] = last + " " + word;
    else lines.push(word);
  }
  const base = h - title * 0.9 - (lines.length - 1) * title * 1.08;
  const text = lines
    .map((l, i) => `<text x="${w * 0.07}" y="${(base + i * title * 1.08).toFixed(0)}" font-size="${title}" font-weight="700" fill="#fff">${esc(l)}</text>`)
    .join("");
  const tag = Math.round(Math.min(w, h) * (portrait ? 0.042 : 0.085));
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}" font-family="Inter, Helvetica, Arial, sans-serif">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 55% 34%)"/><stop offset="1" stop-color="hsl(${hue2} 60% 16%)"/></linearGradient>
<linearGradient id="f" x1="0" y1="0" x2="0" y2="1"><stop offset="0.45" stop-color="#000" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity="0.6"/></linearGradient></defs>
<rect width="${w}" height="${h}" fill="url(#g)"/>${shapes}<rect width="${w}" height="${h}" fill="url(#f)"/>${text}
<text x="${w * 0.07}" y="${tag * 1.9}" font-size="${tag}" fill="#fff" opacity="0.75">sample art</text></svg>`;
}

function icon(id) {
  const r = rng(id * 104729);
  const hue = Math.floor(r() * 360);
  const name = NAMES[id] || `App ${id}`;
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64" font-family="Inter, Helvetica, Arial, sans-serif">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 65% 52%)"/><stop offset="1" stop-color="hsl(${(hue + 50) % 360} 70% 26%)"/></linearGradient></defs>
<rect width="64" height="64" fill="url(#g)"/><circle cx="${18 + r() * 28}" cy="${16 + r() * 20}" r="${10 + r() * 10}" fill="#fff" opacity="0.22"/>
<text x="32" y="45" text-anchor="middle" font-size="36" font-weight="800" fill="#fff">${esc(name[0])}</text></svg>`;
}

function send(res, status, type, body) {
  res.writeHead(status, { "content-type": type, "cache-control": "no-store", "access-control-allow-origin": "*" });
  res.end(body);
}

http
  .createServer((req, res) => {
    const url = new URL(req.url, `http://localhost:${port}`);
    const route = url.pathname;
    let m;
    if (route.startsWith("/IStoreBrowseService/GetItems/v1")) {
      let input;
      try {
        input = JSON.parse(url.searchParams.get("input_json"));
      } catch {
        log(`API bad input`);
        return send(res, 400, "application/json", "{}");
      }
      const ids = input.ids.map((x) => x.appid);
      log(`API ${ids.length} ids include_assets=${input.data_request?.include_assets} lang=${input.context?.language} [${ids.slice(0, 8).join(",")}${ids.length > 8 ? ",…" : ""}]`);
      const store_items = ids.map((id) => {
        if (DELISTED.has(id)) return { item_type: 0, id, success: 15, visible: false, appid: 0 };
        const dir = HASHED.has(id) ? `${hashOf(id)}/` : "";
        const iconHash = `${hashOf(id + 1)}${hashOf(id + 2)}`;
        const assets = {
          asset_url_format: `steam/apps/${id}/\${FILENAME}?t=1700000000`,
          header: `${dir}header.jpg`,
          // IdleKit accepts the icon as a bare hash or as a file name; serve some of each.
          community_icon: Math.floor(id / 4) % 2 ? iconHash : `${iconHash}.jpg`,
        };
        if (!NO_COVER.has(id)) assets.library_capsule = HASHED.has(id) ? `${dir}library_capsule.jpg` : "library_600x900.jpg";
        return { item_type: 0, id, success: 1, visible: true, name: NAMES[id] || `App ${id}`, appid: id, assets };
      });
      return setTimeout(() => send(res, 200, "application/json", JSON.stringify({ response: { store_items } })), 150);
    }
    if ((m = route.match(/^\/store_item_assets\/steam\/apps\/(\d+)\/(?:([0-9a-f]{8})\/)?(header|library_600x900|library_capsule)\.jpg$/))) {
      const id = Number(m[1]);
      const cover = m[3] !== "header";
      // With FAKE_STEAM_STRICT set, hashed games answer only on their hashed path (as on Steam).
      const hashedOk = HASHED.has(id) ? m[2] === hashOf(id) || (!m[2] && !process.env.FAKE_STEAM_STRICT) : !m[2];
      const ok = hashedOk && !NO_ART.has(id) && !(cover && NO_COVER.has(id));
      log(`IMG ${ok ? 200 : 404} ${route}`);
      if (!ok) return send(res, 404, "text/plain", "not found");
      return send(res, 200, "image/svg+xml", cover ? art(id, 600, 900) : art(id, 460, 215));
    }
    if ((m = route.match(/^\/community_assets\/images\/apps\/(\d+)\/[^/]+\.jpg$/))) {
      const id = Number(m[1]);
      const ok = !NO_ART.has(id) && !NO_ICON.has(id);
      log(`IMG ${ok ? 200 : 404} ${route}`);
      if (!ok) return send(res, 404, "text/plain", "not found");
      return send(res, 200, "image/svg+xml", icon(id));
    }
    log(`??? 404 ${route}`);
    send(res, 404, "text/plain", "not found");
  })
  .listen(port, "127.0.0.1", () => console.log(`Stand-in Steam on http://127.0.0.1:${port} (request log: ${LOG})`));
