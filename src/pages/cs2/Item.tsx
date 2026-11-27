// One item of the CS2 market, the way Steam's own item page shows it: what
// it is, the kinds it comes in, what is for sale and what buyers offer, its
// listings with wear, pattern and stickers, and what it sold for over time.
import { ArrowLeft, ExternalLink, RefreshCw, ShoppingCart, Tag } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button, Callout, Empty, Pill, cx } from "../../components/ui";
import { api, openExternal } from "../../lib/api";
import { orderedTags, useCs2Store } from "../../lib/cs2";
import { money, plural } from "../../lib/format";
import type { Cs2Depth, Cs2ItemPage, Cs2Level, Cs2Listing } from "../../lib/types";
import { BuyListingDialog, BuyOrderDialog } from "./Buy";
import { Chips, ItemIcon, ItemName, PriceChart, Wear, useAsked } from "./parts";

/** What is known of an item when its page is opened: from a search result, or only a name from the search box. */
export interface OpenItem {
  /** What its listings are asked for by. */
  id: string;
  name: string;
  /** Its market name, when it has only one. */
  hashName: string | null;
  kind: string;
  iconUrl: string;
  color: string;
  commodity: boolean | null;
}

/** The kinds a group's listings can be narrowed to. */
const KIND_FACETS = ["Exterior", "Quality"];

/** How often an open item's order book is read again. */
const BOOK_EVERY_MS = 20_000;

function Ladder({ title, levels, more, currency, empty }: { title: string; levels: Cs2Level[]; more: number; currency: number; empty: string }) {
  let running = 0;
  return (
    <div className="min-w-0 flex-1 overflow-hidden rounded-xl border border-line bg-panel">
      <div className="border-b border-line px-4 py-2 text-[13px] font-medium">{title}</div>
      {levels.length === 0 ? (
        <div className="px-4 py-4 text-[13px] text-muted">{empty}</div>
      ) : (
        <table className="w-full text-left text-[13px]">
          <thead className="text-[12px] text-faint">
            <tr>
              <th className="px-4 py-1.5 font-normal">Price</th>
              <th className="px-3 py-1.5 text-right font-normal">Quantity</th>
              <th className="px-4 py-1.5 text-right font-normal">In all</th>
            </tr>
          </thead>
          <tbody>
            {levels.slice(0, 8).map((l) => {
              running += l.quantity;
              return (
                <tr key={l.price} className="border-t border-line">
                  <td className="num px-4 py-1.5">{money(l.price, currency)}</td>
                  <td className="num px-3 py-1.5 text-right text-muted">{l.quantity.toLocaleString()}</td>
                  <td className="num px-4 py-1.5 text-right text-faint">{running.toLocaleString()}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {levels.length + more > 8 && <div className="border-t border-line px-4 py-1.5 text-[12px] text-faint">and {(levels.length + more - 8).toLocaleString()} more price(s)</div>}
    </div>
  );
}

function Summary({ depth, loading, error }: { depth: Cs2Depth | null | undefined; loading: boolean; error: string | null }) {
  if (error) return <Callout tone="bad">The order book couldn't be read: {error}</Callout>;
  if (depth === undefined) return <div className="text-[13px] text-muted">{loading ? "Reading the order book…" : ""}</div>;
  if (depth === null) return <Callout>Steam has no market for this item right now.</Callout>;
  return (
    <div className="flex flex-wrap gap-x-8 gap-y-2 text-[13px]">
      <div>
        <span className="num text-[15px] font-semibold">{depth.sellCount.toLocaleString()}</span> <span className="text-muted">for sale</span>
        {depth.lowestSell !== null && (
          <>
            <span className="text-muted"> starting at </span>
            <span className="num text-[15px] font-semibold">{money(depth.lowestSell, depth.currency)}</span>
          </>
        )}
      </div>
      <div>
        <span className="num text-[15px] font-semibold">{depth.buyCount.toLocaleString()}</span> <span className="text-muted">requests to buy</span>
        {depth.highestBuy !== null && (
          <>
            <span className="text-muted"> at </span>
            <span className="num text-[15px] font-semibold">{money(depth.highestBuy, depth.currency)}</span>
            <span className="text-muted"> or lower</span>
          </>
        )}
      </div>
    </div>
  );
}

function ListingRowView({ l, onBuy }: { l: Cs2Listing; onBuy: () => void }) {
  return (
    <tr className="border-t border-line first:border-t-0">
      <td className="px-4 py-2">
        <div className="flex items-center gap-3">
          <ItemIcon src={l.image ?? l.iconUrl} size={56} />
          <div className="min-w-0">
            <div className="flex">
              <ItemName name={l.variant || l.name} color={l.variant ? undefined : l.color} />
            </div>
            {l.stickers.length > 0 && (
              <div className="mt-1 flex items-center gap-1">
                {l.stickers.map((s, i) => (
                  <span key={i} title={`${s.name}${s.wear !== null && s.wear > 0 ? ` (scraped ${Math.round(s.wear * 100)}%)` : ""}`}>
                    <ItemIcon src={s.iconUrl} size={24} />
                  </span>
                ))}
                <span className="ml-1 truncate text-[12px] text-faint">{l.stickers.length === 1 ? l.stickers[0].name : `${l.stickers.length} stickers or charms`}</span>
              </div>
            )}
          </div>
        </div>
      </td>
      <td className="px-3 py-2">{l.float !== null ? <Wear float={l.float} /> : <span className="text-faint">—</span>}</td>
      <td className="num px-3 py-2 text-right text-muted">{l.pattern ?? "—"}</td>
      <td className="px-3 py-2 text-right">
        <div className="num font-medium" title={`${money(l.price, l.currency)} to the seller, ${money(l.fee, l.currency)} in fees`}>
          {money(l.total, l.currency)}
        </div>
        <div className="num text-[12px] text-faint">{money(l.price, l.currency)} + fees</div>
      </td>
      <td className="px-4 py-2">
        <div className="flex items-center justify-end gap-2">
          {l.inspect && (
            <Button size="sm" variant="ghost" icon={<ExternalLink size={13} />} onClick={() => void openExternal(l.inspect!)} title="Opens this very item in Counter-Strike 2">
              Inspect
            </Button>
          )}
          {l.mine ? (
            <Pill>Your listing</Pill>
          ) : (
            <Button size="sm" variant="primary" icon={<ShoppingCart size={13} />} onClick={onBuy} aria-label={`Buy for ${money(l.total, l.currency)}`}>
              Buy
            </Button>
          )}
        </div>
      </td>
    </tr>
  );
}

export function ItemView({ steamId, item, onBack }: { steamId: string; item: OpenItem; onBack: () => void }) {
  const [picked, setPicked] = useState<Record<string, string[]>>({});
  const [more, setMore] = useState<Cs2ItemPage[]>([]);
  const [loadingMore, setLoadingMore] = useState(false);
  const [buying, setBuying] = useState<Cs2Listing | null>(null);
  const [ordering, setOrdering] = useState<"buy" | "order" | null>(null);
  const [allLines, setAllLines] = useState(false);
  const [moreError, setMoreError] = useState<string | null>(null);
  const facets = useCs2Store((s) => s.facets);

  // A case or a key has no listings of its own: it sells from its order book.
  const askListings = item.commodity !== true;
  const pageKey = askListings ? `${item.id}|${JSON.stringify(picked)}` : null;
  const first = useAsked(pageKey, () => api.cs2Item(steamId, item.id, picked, 0));
  // What is looked at now, for an answer that arrives after it changed.
  const latestKey = useRef(pageKey);
  latestKey.current = pageKey;
  useEffect(() => {
    setMore([]);
    setMoreError(null);
  }, [pageKey]);
  const pages = useMemo(() => (first.data ? [first.data, ...more] : []), [first.data, more]);
  const listings = useMemo(() => pages.flatMap((p) => p.listings), [pages]);
  const lastPage = pages[pages.length - 1];

  // The kind in view: the one picked, else the cheapest listing's. It stays
  // while other listings are on their way, so nothing is asked for twice.
  const [hash, setHash] = useState<string | null>(item.hashName ?? (askListings ? null : item.name));
  useEffect(() => {
    if (!first.data) return;
    const narrowed = Object.keys(picked).length > 0;
    setHash(first.data.listings[0]?.hashName || (narrowed ? null : (item.hashName ?? item.name)));
    // When the listings arrive; `picked` is what they were asked with.
  }, [first.data]);
  const about = useAsked(hash ? `about|${hash}` : null, () => api.cs2About(steamId, hash!));
  const book = useAsked(hash ? `book|${hash}` : null, () => api.cs2Book(steamId, hash!));
  const history = useAsked(hash ? `history|${hash}` : null, () => api.cs2History(steamId, hash!));
  const commodity = item.commodity ?? about.data?.commodity ?? (first.data ? first.data.listings.length === 0 && first.data.total <= 1 : false);

  // The order book moves: it is read again every so often while the item is open and the window is looked at.
  useEffect(() => {
    if (!hash) return;
    const timer = setInterval(() => {
      if (document.visibilityState === "visible" && !buying && !ordering) book.reload();
    }, BOOK_EVERY_MS);
    return () => clearInterval(timer);
    // `book.reload` is stable; the timer restarts with the kind in view.
  }, [hash, buying, ordering]);

  const kinds = useMemo(() => {
    const counts = first.data?.facets ?? [];
    return KIND_FACETS.map((facet) => {
      const tags = counts.filter((c) => c.facet === facet);
      const order = facets?.find((f) => f.id === facet);
      const rank = order ? new Map(orderedTags(order).map((t, i) => [t.id, i])) : null;
      const sorted = [...tags].sort((a, b) => (rank ? (rank.get(a.tag) ?? 99) - (rank.get(b.tag) ?? 99) : a.tag.localeCompare(b.tag)));
      return { facet, name: sorted[0]?.facetName ?? facet, tags: sorted };
    }).filter((k) => k.tags.length > 1 || (picked[k.facet]?.length ?? 0) > 0);
  }, [first.data, facets, picked]);

  const toggle = (facet: string, tag: string) =>
    setPicked((p) => {
      const next = { ...p };
      if (p[facet]?.[0] === tag) delete next[facet];
      else next[facet] = [tag];
      return next;
    });

  const loadMore = async () => {
    if (!lastPage || loadingMore) return;
    setLoadingMore(true);
    const key = pageKey;
    try {
      const page = await api.cs2Item(steamId, item.id, picked, listings.length);
      // Only if the same listings are still looked at.
      if (key === latestKey.current) {
        setMore((m) => [...m, page]);
        setMoreError(null);
      }
    } catch (e) {
      // The reason shows where the listings end.
      setMoreError(String(e));
    } finally {
      setLoadingMore(false);
    }
  };

  const refresh = () => {
    first.reload();
    book.reload();
  };
  const name = about.data?.groupName && !commodity && Object.keys(picked).length === 0 ? item.name : (about.data?.name ?? item.name);
  const icon = about.data?.iconUrl || item.iconUrl;
  const lines = about.data?.lines ?? [];
  const holdDays = about.data?.holdDays ?? 0;
  const depth = book.data;
  const grouped = !commodity && kinds.length > 0;

  return (
    <div>
      <div className="mb-4 flex items-center justify-between gap-3">
        <Button size="sm" variant="ghost" icon={<ArrowLeft size={14} />} onClick={onBack}>
          Back to results
        </Button>
        <Button size="sm" icon={<RefreshCw size={13} className={cx((first.loading || book.loading) && "animate-spin")} />} disabled={first.loading || book.loading} onClick={refresh}>
          Refresh
        </Button>
      </div>

      <div className="flex flex-wrap items-start gap-5">
        <ItemIcon src={icon} size={128} className="rounded-xl" />
        <div className="min-w-0 flex-1">
          <h2 className="heading flex text-[22px] font-semibold leading-tight">
            <ItemName name={name} color={about.data?.color ?? item.color} />
          </h2>
          <div className="mt-1 text-[13px] text-muted">
            {[about.data?.kind || item.kind, "Counter-Strike 2"].filter(Boolean).join(" · ")}
            {holdDays > 0 && <span className="text-faint"> · can't be traded or sold on for {holdDays} days after buying</span>}
          </div>
          {lines.length > 0 && (
            <div className="mt-2 max-w-[78ch] text-[13px] text-muted">
              {(allLines ? lines : lines.slice(0, 2)).map((line, i) => (
                <p key={i} className="mt-1 first:mt-0">
                  {line}
                </p>
              ))}
              {lines.length > 2 && (
                <button type="button" className="mt-1 text-faint underline underline-offset-2 hover:text-ink" onClick={() => setAllLines((v) => !v)}>
                  {allLines ? "Show less" : `Show ${lines.length - 2} more line(s)`}
                </button>
              )}
            </div>
          )}
        </div>
      </div>

      {grouped && (
        <div className="mt-5 flex flex-col gap-2">
          {kinds.map((k) => (
            <Chips
              key={k.facet}
              label={k.name}
              options={k.tags.map((t) => ({ id: t.tag, name: t.tagName, note: t.listings.toLocaleString(), color: t.color }))}
              picked={picked[k.facet] ?? []}
              onToggle={(tag) => toggle(k.facet, tag)}
            />
          ))}
        </div>
      )}

      <div className="mt-5 flex flex-wrap items-center justify-between gap-4 rounded-xl border border-line bg-panel px-4 py-3">
        <div className="min-w-0">
          <Summary depth={depth} loading={book.loading || first.loading} error={book.error} />
          {grouped && hash && (
            <div className="mt-1 truncate text-[12px] text-faint">
              For {hash}
              {Object.keys(picked).length === 0 ? ", the cheapest kind for sale. Pick a kind above to see another's." : "."}
            </div>
          )}
        </div>
        <div className="flex flex-none gap-2">
          {commodity && (
            <Button variant="primary" icon={<ShoppingCart size={14} />} disabled={!depth || depth.lowestSell === null} onClick={() => setOrdering("buy")}>
              Buy…
            </Button>
          )}
          <Button icon={<Tag size={14} />} disabled={!hash || depth === undefined} onClick={() => setOrdering("order")}>
            Place buy order…
          </Button>
        </div>
      </div>

      <div className="mt-5">
        {history.data ? (
          <PriceChart history={history.data} name={hash ?? name} />
        ) : (
          <div className="text-[13px] text-muted">{history.error ? `The price history couldn't be read: ${history.error}` : history.loading ? "Reading the price history…" : hash && history.data === null ? "Steam has no price history for this item." : ""}</div>
        )}
      </div>

      {commodity ? (
        depth && (
          <div className="mt-5 flex flex-wrap gap-4">
            <Ladder title="For sale" levels={depth.sells} more={depth.moreSells} currency={depth.currency} empty="Nobody is selling this right now." />
            <Ladder title="Requests to buy" levels={depth.buys} more={depth.moreBuys} currency={depth.currency} empty="Nobody has an order in for this right now." />
          </div>
        )
      ) : (
        <div className="mt-5">
          {first.error ? (
            <Callout tone="bad">
              <div className="font-medium">The listings couldn't be read</div>
              <div className="mt-1">{first.error}</div>
              <div className="mt-3">
                <Button size="sm" onClick={first.reload}>
                  Try again
                </Button>
              </div>
            </Callout>
          ) : !first.data ? (
            <Empty title="Reading the listings…" body="Steam sends these twenty at a time, cheapest first." />
          ) : listings.length === 0 ? (
            <Empty title="None for sale" body={Object.keys(picked).length ? "No listing matches the kind picked. Pick another above." : "Nobody is selling this right now. A buy order waits for the next seller."} />
          ) : (
            <>
              <div className="mb-2 text-[12px] text-faint">
                Showing {listings.length.toLocaleString()} of {plural(Math.max(first.data.total, listings.length), "listing")}, cheapest first
              </div>
              <div className="overflow-x-auto rounded-xl border border-line bg-panel">
                <table className="w-full text-left text-[13px]">
                  <thead className="border-b border-line text-[12px] text-faint">
                    <tr>
                      <th className="px-4 py-2.5 font-normal">Item</th>
                      <th className="px-3 py-2.5 font-normal">Wear</th>
                      <th className="px-3 py-2.5 text-right font-normal">Pattern</th>
                      <th className="px-3 py-2.5 text-right font-normal">Price</th>
                      <th className="px-4 py-2.5" />
                    </tr>
                  </thead>
                  <tbody>
                    {listings.map((l) => (
                      <ListingRowView key={l.listingId} l={l} onBuy={() => setBuying(l)} />
                    ))}
                  </tbody>
                </table>
              </div>
              {moreError && <div className="mt-2 text-[13px] text-bad">More listings couldn't be read: {moreError}</div>}
              {lastPage?.more && listings.length < first.data.total && (
                <div className="mt-3 flex justify-center">
                  <Button busy={loadingMore} onClick={() => void loadMore()}>
                    Show more listings
                  </Button>
                </div>
              )}
            </>
          )}
        </div>
      )}

      {buying && (
        <BuyListingDialog
          steamId={steamId}
          listing={buying}
          holdDays={holdDays}
          onClose={() => setBuying(null)}
          onBought={() => {
            // What was bought is no longer for sale.
            first.reload();
            book.reload();
          }}
        />
      )}
      {ordering && hash && (
        <BuyOrderDialog
          steamId={steamId}
          hashName={hash}
          name={about.data?.name ?? item.name}
          iconUrl={icon}
          depth={depth ?? null}
          buyNow={ordering === "buy"}
          onClose={() => setOrdering(null)}
          onPlaced={() => book.reload()}
        />
      )}
    </div>
  );
}
