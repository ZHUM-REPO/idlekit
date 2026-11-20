import { useEffect, useReducer, useState, type CSSProperties } from "react";
import { markFailed, markLoaded, pick, requestArt, useArtStore, type ArtEntry, type ArtKind } from "../lib/art";
import { cx } from "./ui";

function hue(appid: number) {
  return (appid * 47) % 360;
}

/** A game's looked-up art; asks for it on first use. */
function useArt(appid: number) {
  const art = useArtStore((s) => s.byApp[appid]);
  useEffect(() => {
    if (!art) requestArt(appid);
  }, [appid, art]);
  return art;
}

/**
 * Whether to start on guessable URLs. The lookup is usually answered from the
 * cache within a few milliseconds, so it gets a short head start; after that,
 * guessing beats waiting on a slow or unreachable Steam.
 */
function useGuessing(art: ArtEntry) {
  const [waited, setWaited] = useState(false);
  useEffect(() => {
    if (art !== undefined) return;
    const timer = setTimeout(() => setWaited(true), 350);
    return () => clearTimeout(timer);
  }, [art]);
  return art !== undefined || waited;
}

function Picture({
  kind,
  appid,
  src,
  className,
  style,
  onFailed,
  backdrop,
}: {
  kind: ArtKind;
  appid: number;
  src: string;
  className: string;
  style?: CSSProperties;
  onFailed: () => void;
  /** A second copy shown only for effect; it doesn't decide which URL is used. */
  backdrop?: boolean;
}) {
  return (
    <img
      src={src}
      alt=""
      aria-hidden={backdrop || undefined}
      loading="lazy"
      decoding="async"
      draggable={false}
      referrerPolicy="no-referrer"
      onLoad={backdrop ? undefined : () => markLoaded(kind, appid, src)}
      onError={
        backdrop
          ? undefined
          : () => {
              markFailed(src);
              onFailed();
            }
      }
      className={className}
      style={style}
    />
  );
}

/**
 * A game's picture from Steam: the wide banner, or the portrait cover used in
 * the Steam library. Falls back through every known source, and finally to a
 * tile with the game's name so it can still be recognized.
 */
export function GameArt({
  appid,
  name,
  shape = "banner",
  className,
}: {
  appid: number;
  name: string;
  shape?: "banner" | "cover";
  className?: string;
}) {
  const art = useArt(appid);
  const guess = useGuessing(art);
  const [, tryNext] = useReducer((n: number) => n + 1, 0);

  const cover = shape === "cover" ? pick("cover", appid, art, guess) : undefined;
  const header =
    shape === "banner"
      ? pick("header", appid, art, guess)
      : // In a cover slot the banner is a stand-in, used only once the lookup has had its say.
        !cover && art !== undefined
        ? pick("header", appid, art)
        : undefined;
  const nothing = !cover && !header && art !== undefined;

  return (
    <div
      className={cx(
        "@container relative w-full overflow-hidden bg-well",
        shape === "cover" ? "aspect-[2/3]" : "aspect-[460/215]",
        className,
      )}
      style={
        nothing
          ? { background: `linear-gradient(160deg, oklch(0.36 0.06 ${hue(appid)}), oklch(0.22 0.04 ${hue(appid) + 40}))` }
          : undefined
      }
    >
      {cover && (
        <Picture key={cover} kind="cover" appid={appid} src={cover} onFailed={tryNext} className="absolute inset-0 size-full object-cover" />
      )}
      {!cover && header && shape === "banner" && (
        <Picture key={header} kind="header" appid={appid} src={header} onFailed={tryNext} className="absolute inset-0 size-full object-cover" />
      )}
      {!cover && header && shape === "cover" && (
        // No portrait exists: show the whole banner over a blurred copy of itself, as the Steam library does.
        <>
          <Picture
            key={`${header}#backdrop`}
            backdrop
            kind="header"
            appid={appid}
            src={header}
            onFailed={tryNext}
            className="absolute inset-0 size-full scale-150 object-cover opacity-70 blur-xl"
          />
          <Picture key={header} kind="header" appid={appid} src={header} onFailed={tryNext} className="absolute inset-0 size-full object-contain" />
        </>
      )}
      {nothing && (
        <div className="absolute inset-0 flex items-end p-[5.5cqw]">
          <span
            className={cx(
              "heading font-semibold leading-[1.15] text-white/90",
              shape === "cover"
                ? "line-clamp-4 text-[length:clamp(12px,9.5cqw,18px)]"
                : "line-clamp-2 text-[length:clamp(11px,7.4cqw,16px)]",
            )}
          >
            {name}
          </span>
        </div>
      )}
    </div>
  );
}

function initials(name: string) {
  return name
    .replace(/[^\p{L}\p{N} ]/gu, "")
    .split(" ")
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => [...w][0])
    .join("")
    .toUpperCase();
}

/** The small square icon Steam shows beside a game's name. */
export function GameIcon({ appid, name, size = 32 }: { appid: number; name: string; size?: number }) {
  const art = useArt(appid);
  const [, tryNext] = useReducer((n: number) => n + 1, 0);

  const icon = pick("icon", appid, art);
  // Some games have no icon on record. Once the lookup has answered, a square
  // crop of the cover still identifies the game better than letters do.
  const crop = !icon && art !== undefined ? pick("cover", appid, art) : undefined;
  const src = icon ?? crop;
  const box = { width: size, height: size };

  if (src) {
    return (
      <Picture
        key={src}
        kind={icon ? "icon" : "cover"}
        appid={appid}
        src={src}
        onFailed={tryNext}
        className="flex-none rounded-md bg-well object-cover"
        style={box}
      />
    );
  }
  if (art === undefined) {
    return <span className="flex-none rounded-md bg-well" style={box} aria-hidden="true" />;
  }
  return (
    <span
      className="inline-flex flex-none items-center justify-center rounded-md text-[11px] font-semibold text-white/85"
      style={{ ...box, background: `oklch(0.38 0.07 ${hue(appid)})` }}
      aria-hidden="true"
    >
      {initials(name) || "?"}
    </span>
  );
}
