// The Counter-Strike 2 market: Steam's own market for the game's items,
// with its search and filters laid out as on Steam's page, an item's
// listings and order book, selling from the CS2 inventory, the account's
// listings and buy orders, and trade offers.
import { RefreshCw, Search, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button, Callout, Empty, PageHeader, Select, Tabs, cx, inputClass, smallInputClass } from "../components/ui";
import { api } from "../lib/api";
import {
  CLASSES,
  EMPTY_FORM,
  QUICK_TYPES,
  RESULTS_PER_PAGE,
  SORTS,
  dependentFacets,
  orderedTags,
  otherTypes,
  pruned,
  tagName,
  toQuery,
  useCs2,
  useCs2Store,
  type SearchForm,
} from "../lib/cs2";
import { currencyCode, plural } from "../lib/format";
import { useKept } from "../lib/kept";
import { useActiveAccount, useApp } from "../lib/store";
import type { Account, Cs2Facet, Cs2Hit, Cs2Suggestion } from "../lib/types";
import { ItemView, type OpenItem } from "./cs2/Item";
import { OrdersTab } from "./cs2/Orders";
import { Chips, ItemIcon, ItemName } from "./cs2/parts";
import { SellTab } from "./cs2/Sell";
import { TradesTab } from "./cs2/Trades";

type Tab = "browse" | "inventory" | "orders" | "trades";

export function Cs2MarketPage() {
  const account = useActiveAccount();
  const go = useApp((s) => s.go);
  const [tab, setTab] = useKept<Tab>("cs2.tab", "browse");
  const steamId = account?.steamId;
  const noWeb = !account || (!account.hasToken && !account.hasCookie);
  const data = useCs2(steamId ?? "");
  // Steam's list of filters is read once; the page works without it too.
  useEffect(() => {
    if (steamId && !noWeb) void useCs2Store.getState().loadFacets(steamId);
  }, [steamId, noWeb]);
  if (!account) return null;
  return (
    <div className="pb-10">
      <PageHeader
        title="CS2 Market"
        sub="Counter-Strike 2 on the Steam Community Market: cases, keys, skins, stickers and the rest. Browse and buy with your Steam Wallet, sell from your inventory, and answer trade offers."
      />
      {noWeb ? (
        <Callout tone="run">
          <div className="font-medium">The market needs a web session for this account</div>
          <div className="mt-1 text-muted">Sign in with a QR code or password, or paste a steamLoginSecure cookie, from Accounts.</div>
          <div className="mt-3">
            <Button size="sm" onClick={() => go("accounts")}>
              Set up web session
            </Button>
          </div>
        </Callout>
      ) : (
        <>
          <div className="mb-5">
            <Tabs
              value={tab}
              onChange={setTab}
              tabs={[
                { value: "browse", label: "Browse" },
                { value: "inventory", label: "Sell", count: data.inventory?.filter((i) => i.marketable).length },
                { value: "orders", label: "My orders", count: data.mine ? data.mine.listings.length + data.mine.orders.length : undefined },
                { value: "trades", label: "Trade offers", count: data.offers?.received.length },
              ]}
            />
          </div>
          {tab === "browse" && <Browse account={account} />}
          {tab === "inventory" && <SellTab account={account} />}
          {tab === "orders" && <OrdersTab account={account} />}
          {tab === "trades" && <TradesTab account={account} />}
        </>
      )}
    </div>
  );
}

/** What a search answered, kept so that coming back to the page asks nothing again. */
interface Found {
  /** The question it answers. */
  key: string;
  hits: Cs2Hit[];
  /** How many results Steam has sent so far: where the next ones start. */
  received: number;
  total: number;
  walletCurrency: number;
  /** Descriptions were asked for and Steam searched names only. */
  namesOnly: boolean;
}

/** How long typing rests before the search is asked. */
const SEARCH_AFTER_MS = 450;
const SUGGEST_AFTER_MS = 300;

function selectClass(on: boolean) {
  return cx(smallInputClass, "max-w-[190px] cursor-pointer pr-1", on && "border-ink text-ink");
}

/** One of the weapon classes: all of it, or one of its weapons. */
function ClassSelect({
  cls,
  facets,
  filters,
  onPick,
}: {
  cls: (typeof CLASSES)[number];
  facets: Cs2Facet[] | null;
  filters: Record<string, string[]>;
  onPick: (kind: { type?: string; weapon?: string } | null) => void;
}) {
  const weapon = (filters.Weapon ?? []).find((w) => cls.weapons.includes(w));
  const value = weapon ?? ((filters.Type ?? []).includes(cls.type) ? "all" : "");
  return (
    <select
      aria-label={cls.label}
      className={selectClass(value !== "")}
      value={value}
      onChange={(e) => onPick(e.target.value === "" ? null : e.target.value === "all" ? { type: cls.type } : { weapon: e.target.value })}
    >
      <option value="">{cls.label}</option>
      <option value="all">{cls.all}</option>
      {cls.weapons.map((w) => (
        <option key={w} value={w}>
          {tagName(facets, "Weapon", w)}
        </option>
      ))}
    </select>
  );
}

function Browse({ account }: { account: Account }) {
  const steamId = account.steamId;
  const facets = useCs2Store((s) => s.facets);
  const facetsFailed = useCs2Store((s) => s.facetsFailed);
  const [form, setForm] = useKept<SearchForm>("cs2.form", EMPTY_FORM);
  const [found, setFound] = useKept<Found | null>(`cs2.found.${steamId}`, null);
  const [open, setOpen] = useKept<OpenItem | null>(`cs2.open.${steamId}`, null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [suggestions, setSuggestions] = useState<Cs2Suggestion[]>([]);
  const [suggesting, setSuggesting] = useState(false);
  const asked = useRef(0);

  const filters = useMemo(() => pruned(form.filters), [form.filters]);
  const key = useMemo(() => JSON.stringify(toQuery(form, 0)), [form]);

  const ask = async (start: number, why: "new" | "more" | "again") => {
    const mine = ++asked.current;
    setLoading(true);
    setError(null);
    try {
      const r = await api.cs2Search(steamId, toQuery(form, start));
      if (mine !== asked.current) return;
      setFound((had) => ({
        key,
        hits: why === "more" && had?.key === key ? [...had.hits, ...r.results.filter((x) => !had.hits.some((h) => h.id === x.id))] : r.results,
        received: (why === "more" && had?.key === key ? had.received : 0) + r.results.length,
        total: r.total,
        walletCurrency: r.walletCurrency,
        namesOnly: form.descriptions && form.text.trim() !== "" && !r.inDescriptions,
      }));
    } catch (e) {
      if (mine === asked.current) setError(String(e));
    } finally {
      if (mine === asked.current) setLoading(false);
    }
  };

  // A changed search is asked once typing rests; one already answered isn't asked again.
  useEffect(() => {
    if (found?.key === key) return;
    const timer = setTimeout(() => void ask(0, "new"), SEARCH_AFTER_MS);
    return () => clearTimeout(timer);
    // `ask` reads the form the key was made from.
  }, [key]);

  // Names for the search box, a moment after typing rests.
  const typed = form.text.trim();
  useEffect(() => {
    if (!suggesting || typed.length < 3) {
      setSuggestions([]);
      return;
    }
    let current = true;
    const timer = setTimeout(() => {
      api.cs2Suggest(steamId, typed).then(
        (list) => current && setSuggestions(list),
        () => current && setSuggestions([]),
      );
    }, SUGGEST_AFTER_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [typed, suggesting, steamId]);

  if (open) return <ItemView key={open.id} steamId={steamId} item={open} onBack={() => setOpen(null)} />;

  const set = (part: Partial<SearchForm>) => setForm((f) => ({ ...f, ...part }));
  const setFilters = (change: (f: Record<string, string[]>) => Record<string, string[]>) => setForm((f) => ({ ...f, filters: pruned(change(f.filters)) }));
  // One kind of item at a time: a class, one of its weapons, or one of the other kinds.
  const pickKind = (kind: { type?: string; weapon?: string } | null) =>
    setFilters((f) => {
      const { Type: _type, Weapon: _weapon, ...rest } = f;
      return kind?.type ? { ...rest, Type: [kind.type] } : kind?.weapon ? { ...rest, Weapon: [kind.weapon] } : rest;
    });
  const toggleTag = (facet: string, tag: string) =>
    setFilters((f) => {
      const had = f[facet] ?? [];
      return { ...f, [facet]: had.includes(tag) ? had.filter((t) => t !== tag) : [...had, tag] };
    });
  const type = (filters.Type ?? [])[0];
  const others = otherTypes(facets);
  const collections = facets?.find((f) => f.id === "ItemSet")?.tags ?? [];
  const narrowed = Object.keys(filters).length > 0 || form.priceMin !== "" || form.priceMax !== "" || typed !== "" || form.descriptions;
  // While a changed search is on its way, the last answer stays in view, dimmed.
  const hits = found?.hits ?? null;
  const total = found?.total ?? 0;
  const current = found?.key === key;
  const otherCurrency = hits?.find((h) => h.currency !== 0 && h.currency !== found!.walletCurrency)?.currency;
  const openHit = (h: Cs2Hit) => setOpen({ id: h.id, name: h.name, hashName: h.hashName, kind: h.kind, iconUrl: h.iconUrl, color: h.color, commodity: h.commodity });

  return (
    <div>
      <div className="flex flex-wrap items-center gap-3">
        <div className="relative min-w-[260px] flex-1">
          <Search size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
          <input
            className={cx(inputClass, "w-full pl-8 pr-8")}
            placeholder="Search Counter-Strike 2 items"
            aria-label="Search Counter-Strike 2 items"
            value={form.text}
            onChange={(e) => set({ text: e.target.value })}
            onFocus={() => setSuggesting(true)}
            onBlur={() => setTimeout(() => setSuggesting(false), 150)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setSuggesting(false);
              if (e.key === "Enter") {
                setSuggesting(false);
                void ask(0, "again");
              }
            }}
          />
          {form.text && (
            <button type="button" aria-label="Clear the search" className="absolute right-2 top-1/2 -translate-y-1/2 text-faint hover:text-ink" onClick={() => set({ text: "" })}>
              <X size={15} />
            </button>
          )}
          {suggesting && suggestions.length > 0 && (
            <div className="absolute left-0 right-0 top-full z-20 mt-1 overflow-hidden rounded-lg border border-line-strong bg-panel shadow-xl" role="listbox" aria-label="Suggestions">
              {suggestions.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  role="option"
                  aria-selected="false"
                  className="flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-[13px] hover:bg-raised"
                  // Before the box loses focus and the list goes.
                  onMouseDown={(e) => {
                    e.preventDefault();
                    setSuggesting(false);
                    setOpen({ id: s.id, name: s.name, hashName: null, kind: "", iconUrl: s.iconUrl, color: "", commodity: null });
                  }}
                >
                  <ItemIcon src={s.iconUrl} size={28} />
                  <span className="truncate">{s.name}</span>
                </button>
              ))}
            </div>
          )}
        </div>
        <label className="flex cursor-pointer items-center gap-2 text-[13px] text-muted">
          <input type="checkbox" className="size-4 accent-[var(--ink)]" checked={form.descriptions} onChange={(e) => set({ descriptions: e.target.checked })} />
          Include descriptions
        </label>
        <Select small label="Sort by" value={form.sort} onChange={(sort) => set({ sort })} options={SORTS.map((s) => ({ value: s.value, label: s.label }))} />
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-1.5" role="group" aria-label="Kind of item">
        {CLASSES.map((cls) => (
          <ClassSelect key={cls.type} cls={cls} facets={facets} filters={filters} onPick={pickKind} />
        ))}
        {QUICK_TYPES.map((q) => {
          const on = type === q.type;
          return (
            <button
              key={q.type}
              type="button"
              aria-pressed={on}
              onClick={() => pickKind(on ? null : { type: q.type })}
              className={cx(
                "inline-flex h-7 items-center rounded-md border px-2.5 text-[12px] transition-colors",
                on ? "border-ink bg-ink text-bg" : "border-line bg-well text-ink hover:border-line-strong",
              )}
            >
              {tagName(facets, "Type", q.type)}
            </button>
          );
        })}
        <select
          aria-label="Other kinds"
          className={selectClass(!!type && others.includes(type))}
          value={type && others.includes(type) ? type : ""}
          onChange={(e) => pickKind(e.target.value ? { type: e.target.value } : null)}
        >
          <option value="">Other</option>
          {others.map((t) => (
            <option key={t} value={t}>
              {tagName(facets, "Type", t)}
            </option>
          ))}
        </select>
        {collections.length > 0 && (
          <select
            aria-label="Collection"
            className={selectClass((filters.ItemSet ?? []).length > 0)}
            value={(filters.ItemSet ?? [])[0] ?? ""}
            onChange={(e) => setFilters((f) => ({ ...f, ItemSet: e.target.value ? [e.target.value] : [] }))}
          >
            <option value="">Collection</option>
            {collections.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
        )}
      </div>

      {facets && dependentFacets(filters).length > 0 && (
        <div className="mt-3 flex flex-col gap-2">
          {dependentFacets(filters).map((id) => {
            const facet = facets.find((f) => f.id === id);
            if (!facet || facet.tags.length === 0) return null;
            const tags = orderedTags(facet);
            // A handful of tags are offered side by side; a long list is picked from.
            return tags.length <= 16 ? (
              <Chips key={id} label={facet.name} options={tags.map((t) => ({ id: t.id, name: t.name, color: t.color }))} picked={filters[id] ?? []} onToggle={(tag) => toggleTag(id, tag)} />
            ) : (
              <div key={id} className="flex items-center gap-2">
                <span className="text-[12px] text-faint">{facet.name}</span>
                <select
                  aria-label={facet.name}
                  className={selectClass((filters[id] ?? []).length > 0)}
                  value={(filters[id] ?? [])[0] ?? ""}
                  onChange={(e) => setFilters((f) => ({ ...f, [id]: e.target.value ? [e.target.value] : [] }))}
                >
                  <option value="">Any</option>
                  {tags.map((t) => (
                    <option key={t.id} value={t.id}>
                      {t.name}
                    </option>
                  ))}
                </select>
              </div>
            );
          })}
        </div>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2 text-[12px] text-muted">
        <span className="text-faint">Price{found ? ` in ${currencyCode(found.walletCurrency)}` : ""}</span>
        <input className={cx(smallInputClass, "num w-24")} inputMode="decimal" placeholder="from" aria-label="Lowest price" value={form.priceMin} onChange={(e) => set({ priceMin: e.target.value })} />
        <span>to</span>
        <input className={cx(smallInputClass, "num w-24")} inputMode="decimal" placeholder="to" aria-label="Highest price" value={form.priceMax} onChange={(e) => set({ priceMax: e.target.value })} />
        {narrowed && (
          <Button size="sm" variant="ghost" icon={<X size={13} />} onClick={() => setForm({ ...EMPTY_FORM, sort: form.sort })}>
            Clear filters
          </Button>
        )}
        <div className="ml-auto flex items-center gap-3">
          {hits && current && (
            <span>
              {total === 0 ? "No items" : `${hits.length.toLocaleString()} of ${plural(Math.max(total, hits.length), "item")}`}
            </span>
          )}
          <Button size="sm" icon={<RefreshCw size={13} className={cx(loading && "animate-spin")} />} disabled={loading} onClick={() => void ask(0, "again")}>
            Refresh
          </Button>
        </div>
      </div>

      {facetsFailed && !facets && (
        <div className="mt-3">
          <Callout>
            Steam's list of filters couldn't be read ({facetsFailed}), so only the kinds above are offered.{" "}
            <button type="button" className="underline underline-offset-2" onClick={() => void useCs2Store.getState().loadFacets(steamId)}>
              Try again
            </button>
          </Callout>
        </div>
      )}
      {current && found?.namesOnly && (
        <div className="mt-3">
          <Callout>Steam gave the search within descriptions no answer, so these are matches by name only.</Callout>
        </div>
      )}
      {otherCurrency !== undefined && (
        <div className="mt-3">
          <Callout>
            Steam's search gave these prices in {currencyCode(otherCurrency)}, not in your wallet's {currencyCode(found!.walletCurrency)}. An item's own page shows your currency.
          </Callout>
        </div>
      )}

      <div className="mt-4">
        {error ? (
          <Callout tone="bad">
            <div className="font-medium">The search couldn't be read</div>
            <div className="mt-1">{error}</div>
            <div className="mt-3">
              <Button size="sm" onClick={() => void ask(0, "again")}>
                Try again
              </Button>
            </div>
          </Callout>
        ) : !hits ? (
          <Empty title="Searching the market…" body="Asking Steam for Counter-Strike 2 items." />
        ) : hits.length === 0 ? (
          <Empty title="Nothing matches" body="No item on the market fits this search. Try fewer filters or other words." />
        ) : (
          <>
            <div className={cx("overflow-hidden rounded-xl border border-line bg-panel transition-opacity", (loading || !current) && "opacity-60")} aria-busy={loading || !current}>
              <div className="flex items-center gap-4 border-b border-line px-4 py-2 text-[12px] text-faint">
                <span className="flex-1">Name</span>
                <span className="w-24 text-right">Quantity</span>
                <span className="w-32 text-right">Starting at</span>
              </div>
              {hits.map((h) => (
                <button
                  key={h.id}
                  type="button"
                  onClick={() => openHit(h)}
                  className="flex w-full items-center gap-4 border-t border-line px-4 py-2 text-left first:border-t-0 hover:bg-raised/60 focus-visible:bg-raised/60 focus-visible:outline-none"
                >
                  <ItemIcon src={h.iconUrl} size={44} />
                  <span className="min-w-0 flex-1">
                    <span className="flex text-[14px]">
                      <ItemName name={h.name} color={h.color} />
                    </span>
                    <span className="block truncate text-[12px] text-faint">
                      {h.kind || "Counter-Strike 2"}
                      {h.grouped ? " · every wear" : ""}
                    </span>
                  </span>
                  <span className="num w-24 text-right text-muted">{h.listings.toLocaleString()}</span>
                  <span className="num w-32 text-right">{h.priceText || "—"}</span>
                </button>
              ))}
            </div>
            {current && found!.received < total && found!.received % RESULTS_PER_PAGE === 0 && (
              <div className="mt-3 flex justify-center">
                <Button busy={loading} onClick={() => void ask(found!.received, "more")}>
                  Show more
                </Button>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
