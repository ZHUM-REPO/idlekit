import { useEffect, useRef, useState, type CSSProperties, type RefObject } from "react";
import { requestArt, sources, useArtStore, type ArtEntry } from "../lib/art";
import { pictureBroken, useFetchPicture, usePictureStatus } from "../lib/pictures";
import { cx } from "./ui";

function hue(appid: number) {
  return (appid * 47) % 360;
}

/** A game's looked-up art. It is only asked for when `wanted`: a picture already on disk needs no lookup. */
function useArt(appid: number, wanted: boolean) {
  const art = useArtStore((s) => s.byApp[appid]);
  useEffect(() => {
    if (wanted && !art) requestArt(appid);
  }, [appid, art, wanted]);
  return art;
}

/**
 * Whether to go ahead with guessable URLs. The lookup is usually answered
 * from the cache within a few milliseconds, so it gets a short head start;
 * after that, guessing beats waiting on a slow or unreachable Steam.
 */
function useGuessing(art: ArtEntry, wanted: boolean) {
  const [waited, setWaited] = useState(false);
  useEffect(() => {
    if (!wanted || art !== undefined) return;
    const timer = setTimeout(() => setWaited(true), 350);
    return () => clearTimeout(timer);
  }, [art, wanted]);
  return art !== undefined || waited;
}

/**
 * One of a game's pictures: the address to show, `null` when there is none,
 * `undefined` while that isn't known. A picture on disk is shown at once.
 * A missing one is fetched in its turn (see pictures.ts) for the tile `el`.
 */
function useGamePicture(kind: "cover" | "header" | "icon", appid: number, el: RefObject<Element | null>, enabled: boolean) {
  const { src, missing } = usePictureStatus(kind, appid, enabled);
  // The icon's address often came with the games list, and then nothing has to be looked up.
  const known = kind === "icon" ? sources("icon", appid, undefined) : [];
  const art = useArt(appid, missing && known.length === 0);
  const guess = useGuessing(art, missing && known.length === 0);
  const ready = known.length > 0 || art !== undefined || guess;
  useFetchPicture(kind, appid, el, missing && ready ? sources(kind, appid, art) : [], {
    enabled: missing,
    ready,
    // Only an answered lookup makes the addresses more than guesses.
    certain: !!art,
  });
  return enabled ? src : undefined;
}

function Picture({
  kind,
  appid,
  src,
  className,
  style,
  backdrop,
}: {
  kind: "cover" | "header" | "icon";
  appid: number;
  src: string;
  className: string;
  style?: CSSProperties;
  /** A second copy shown only for effect. */
  backdrop?: boolean;
}) {
  return (
    <img
      src={src}
      alt=""
      aria-hidden={backdrop || undefined}
      decoding="async"
      draggable={false}
      onError={backdrop ? undefined : () => pictureBroken(kind, appid)}
      className={className}
      style={style}
    />
  );
}

/**
 * A game's picture from Steam: the wide banner, or the portrait cover used in
 * the Steam library. Until the picture is there, and when there is none, the
 * tile shows the game's name so it can still be recognized.
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
  const box = useRef<HTMLDivElement>(null);
  const cover = useGamePicture("cover", appid, box, shape === "cover");
  // In a cover slot the banner is a stand-in, used only once it is known that there is no cover.
  const header = useGamePicture("header", appid, box, shape === "banner" || cover === null);
  const showCover = shape === "cover" && !!cover;
  const showHeader = !showCover && !!header;
  const named = !showCover && !showHeader;

  return (
    <div
      ref={box}
      className={cx(
        "@container relative w-full overflow-hidden bg-well",
        shape === "cover" ? "aspect-[2/3]" : "aspect-[460/215]",
        className,
      )}
      style={
        named
          ? { background: `linear-gradient(160deg, oklch(0.36 0.06 ${hue(appid)}), oklch(0.22 0.04 ${hue(appid) + 40}))` }
          : undefined
      }
    >
      {showCover && <Picture key={cover} kind="cover" appid={appid} src={cover} className="absolute inset-0 size-full object-cover" />}
      {showHeader && shape === "banner" && (
        <Picture key={header} kind="header" appid={appid} src={header} className="absolute inset-0 size-full object-cover" />
      )}
      {showHeader && shape === "cover" && (
        // No portrait exists: show the whole banner over a blurred copy of itself, as the Steam library does.
        <>
          <Picture
            key={`${header}#backdrop`}
            backdrop
            kind="header"
            appid={appid}
            src={header}
            className="absolute inset-0 size-full scale-150 object-cover opacity-70 blur-xl"
          />
          <Picture key={header} kind="header" appid={appid} src={header} className="absolute inset-0 size-full object-contain" />
        </>
      )}
      {named && (
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
  const box = useRef<HTMLSpanElement>(null);
  const icon = useGamePicture("icon", appid, box, true);
  // Some games have no icon on record. A square crop of the cover still
  // identifies the game better than letters do.
  const crop = useGamePicture("cover", appid, box, icon === null);
  const src = icon ?? crop;
  const waiting = icon === undefined || (icon === null && crop === undefined);
  const dims = { width: size, height: size };

  return (
    <span
      ref={box}
      aria-hidden="true"
      className={cx(
        "inline-flex flex-none items-center justify-center overflow-hidden rounded-md text-[11px] font-semibold text-white/85",
        (src || waiting) && "bg-well",
      )}
      style={src || waiting ? dims : { ...dims, background: `oklch(0.38 0.07 ${hue(appid)})` }}
    >
      {src ? (
        <Picture key={src} kind={icon ? "icon" : "cover"} appid={appid} src={src} className="size-full object-cover" />
      ) : waiting ? null : (
        initials(name) || "?"
      )}
    </span>
  );
}
