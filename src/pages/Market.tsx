import { RefreshCw, Search, Tag, Trash } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Button, Callout, Empty, Modal, PageHeader, Pill, Select, Tabs, TextInput, cx, inputClass } from "../components/ui";
import { api } from "../lib/api";
import { HiddenNotice } from "../components/HiddenNotice";
import { money, sellerReceives } from "../lib/format";
import { useAppIds, useHiding } from "../lib/hidden";
import { useKept, useView } from "../lib/kept";
import { useMarket, useMarketStore } from "../lib/market";
import { useActiveAccount, useApp, useEnsureGames } from "../lib/store";
import type { Account, InvItem } from "../lib/types";

const CLASS_LABEL: Record<InvItem["class"], string> = {
  card: "Trading card",
  foil: "Foil card",
  emoticon: "Emoticon",
  background: "Background",
  booster: "Booster pack",
  gems: "Gems",
  other: "Other",
};

export function MarketPage() {
  const account = useActiveAccount();
  const [tab, setTab] = useView("marketTab");
  const go = useApp((s) => s.go);
  const steamId = account?.steamId;
  const noWeb = !account || (!account.hasToken && !account.hasCookie);
  // With a web session, the inventory and the listings load by themselves,
  // once while IdleKit runs; Refresh asks again.
  useEffect(() => {
    if (steamId && !noWeb) useMarketStore.getState().loadByItself(steamId);
  }, [steamId, noWeb]);
  if (!account) return null;
  return (
    <div className="pb-10">
      <PageHeader
        title="Market"
        sub="Sell trading cards, emoticons, backgrounds and booster packs from your Steam inventory, priced from the live order book."
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
                { value: "inventory", label: "Inventory" },
                { value: "listings", label: "My listings" },
              ]}
            />
          </div>
          {tab === "inventory" ? <Inventory account={account} /> : <Listings account={account} />}
        </>
      )}
    </div>
  );
}

function ProgressBar() {
  const p = useApp((s) => s.marketProgress);
  if (!p) return null;
  return (
    <div className="mb-4 flex items-center gap-4 rounded-lg border border-line bg-panel px-4 py-3">
      <span className="lamp on" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px]">{p.message}</div>
        <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-well">
          <div className="h-full bg-run transition-[width]" style={{ width: `${(p.done / Math.max(1, p.total)) * 100}%` }} />
        </div>
      </div>
      <span className="num text-[13px] text-muted">
        {p.done} of {p.total}
      </span>
      <Button size="sm" variant="ghost" onClick={() => void api.marketCancel()}>
        Cancel
      </Button>
    </div>
  );
}

/** Steam's own app id: gems, sale cards and the like belong to no game. */
const STEAM_ITSELF = 753;

/**
 * The game an inventory item comes from. Steam names community items
 * "<the game's app id>-<item name>"; 0 when the name doesn't say.
 */
function gameOf(item: InvItem): number {
  const id = Number(/^(\d+)-/.exec(item.marketHashName)?.[1] ?? 0);
  return id === STEAM_ITSELF ? 0 : id;
}

function Inventory({ account }: { account: Account }) {
  const run = useApp((s) => s.run);
  const settings = useApp((s) => s.settings)!;
  const { items, itemsLoading: loading, prices, wallet } = useMarket(account.steamId);
  const currency = wallet.currency;
  const [qty, setQty] = useState<Record<string, number>>({});
  const [price, setPrice] = useState<Record<string, string>>({});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useView("marketFilter");
  const [query, setQuery] = useKept("market.query", "");
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);

  const load = () => useMarketStore.getState().loadInventory(account.steamId);
  // A fresh inventory starts with nothing selected.
  useEffect(() => setSelected(new Set()), [items]);

  // "Hide NSFW games" goes by the game each item comes from.
  const itemGames = useMemo(() => items?.map(gameOf).filter((id) => id > 0), [items]);
  const hiding = useHiding(itemGames);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (items ?? []).filter(
      (i) =>
        i.marketable &&
        !(gameOf(i) > 0 && hiding.hides(gameOf(i))) &&
        (filter === "all" || i.class === filter) &&
        (!q || i.name.toLowerCase().includes(q) || i.game.toLowerCase().includes(q)),
    );
  }, [items, filter, query, hiding]);

  const unmarketable = (items ?? []).filter((i) => !i.marketable).length;
  const qtyOf = (i: InvItem) => qty[i.marketHashName] ?? i.count;
  const priceOf = (i: InvItem): number | null => {
    const typed = price[i.marketHashName];
    if (typed !== undefined && typed.trim() !== "") {
      const v = Math.round(Number(typed.replace(",", ".")) * 100);
      return Number.isFinite(v) && v > 0 ? v : null;
    }
    return prices[i.marketHashName]?.suggested ?? null;
  };

  const sellable = shown.filter((i) => selected.has(i.marketHashName) && qtyOf(i) > 0 && priceOf(i));
  const totalBuyer = sellable.reduce((n, i) => n + (priceOf(i) ?? 0) * qtyOf(i), 0);
  const totalYou = sellable.reduce((n, i) => n + sellerReceives(priceOf(i) ?? 0, wallet) * qtyOf(i), 0);

  const toggle = (h: string) =>
    setSelected((s) => {
      const n = new Set(s);
      if (n.has(h)) n.delete(h);
      else n.add(h);
      return n;
    });

  if (!items) {
    return loading ? (
      <Empty
        title="Loading your inventory…"
        body="Reading the Steam inventory (cards, emoticons, backgrounds, boosters) for this account. Nothing is listed until you confirm."
      />
    ) : (
      <Empty
        title="Load your inventory"
        body="Reads the Steam inventory (cards, emoticons, backgrounds, boosters) for this account. Nothing is listed until you confirm."
        action={
          <Button variant="primary" onClick={() => void load()}>
            Load inventory
          </Button>
        }
      />
    );
  }

  return (
    <div className="pb-16">
      <ProgressBar />
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <label className="relative">
          <Search size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
          <TextInput value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search items or games" aria-label="Search items" className="w-64 pl-8" />
        </label>
        <Select
          label="Item type"
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: "All marketable" },
            { value: "card", label: "Trading cards" },
            { value: "foil", label: "Foil cards" },
            { value: "emoticon", label: "Emoticons" },
            { value: "background", label: "Backgrounds" },
            { value: "booster", label: "Booster packs" },
            { value: "other", label: "Other" },
          ]}
        />
        <Button
          size="sm"
          onClick={() => {
            const dupes = shown.filter((i) => i.count > 1);
            setSelected(new Set(dupes.map((i) => i.marketHashName)));
            setQty((q) => ({ ...q, ...Object.fromEntries(dupes.map((i) => [i.marketHashName, i.count - 1])) }));
          }}
        >
          Select duplicates, keep one
        </Button>
        <Button size="sm" variant="ghost" onClick={() => setSelected(new Set(shown.map((i) => i.marketHashName)))}>
          Select all
        </Button>
        <div className="ml-auto flex gap-2">
          <Button size="sm" icon={<RefreshCw size={13} className={cx(loading && "animate-spin")} />} disabled={loading} onClick={() => void load()}>
            Refresh
          </Button>
          <Button
            size="sm"
            icon={<Tag size={13} />}
            disabled={selected.size === 0}
            busy={busy === "price"}
            onClick={async () => {
              setBusy("price");
              const r = await run(null, () => api.prices(account.steamId, [...selected]));
              if (r) useMarketStore.getState().priced(account.steamId, r.wallet, r.items);
              setBusy(null);
            }}
          >
            Get prices for {selected.size || ""} selected
          </Button>
        </div>
      </div>

      <HiddenNotice hiding={hiding} />

      {shown.length === 0 ? (
        <Empty title="Nothing to sell here" body="No marketable items match this filter." />
      ) : (
        <div className="overflow-x-auto rounded-xl border border-line bg-panel">
          <table className="w-full text-left text-[13px]">
            <thead className="border-b border-line text-[12px] text-faint">
              <tr>
                <th className="w-10 px-4 py-2.5" />
                <th className="py-2.5 font-normal">Item</th>
                <th className="px-3 py-2.5 text-right font-normal">Owned</th>
                <th className="px-3 py-2.5 text-right font-normal">Sell</th>
                <th className="px-3 py-2.5 text-right font-normal">Lowest listing</th>
                <th className="px-3 py-2.5 text-right font-normal">Highest offer</th>
                <th className="px-3 py-2.5 text-right font-normal">Buyer pays</th>
                <th className="px-4 py-2.5 text-right font-normal">You receive</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((i) => {
                const p = prices[i.marketHashName];
                const sel = selected.has(i.marketHashName);
                const each = priceOf(i);
                return (
                  <tr key={i.marketHashName} className={cx("border-t border-line first:border-t-0", sel && "bg-raised/50")}>
                    <td className="px-4 py-2">
                      <input
                        type="checkbox"
                        checked={sel}
                        onChange={() => toggle(i.marketHashName)}
                        aria-label={`Select ${i.name}`}
                        className="size-4 accent-[var(--ink)]"
                      />
                    </td>
                    <td className="py-2">
                      <div className="flex items-center gap-3">
                        {i.iconUrl ? (
                          <img src={i.iconUrl} alt="" className="size-9 flex-none rounded bg-well object-contain" />
                        ) : (
                          <span className="size-9 flex-none rounded bg-well" aria-hidden="true" />
                        )}
                        <div className="min-w-0">
                          <div className="truncate">{i.name}</div>
                          <div className="truncate text-[12px] text-faint">
                            {CLASS_LABEL[i.class]}
                            {i.game ? `, ${i.game}` : ""}
                          </div>
                        </div>
                      </div>
                    </td>
                    <td className="num px-3 py-2 text-right text-muted">{i.count}</td>
                    <td className="px-3 py-2 text-right">
                      <input
                        aria-label={`How many ${i.name} to sell`}
                        type="number"
                        min={0}
                        max={i.count}
                        value={qtyOf(i)}
                        onChange={(e) =>
                          setQty((q) => ({ ...q, [i.marketHashName]: Math.max(0, Math.min(i.count, Number(e.target.value) || 0)) }))
                        }
                        className={cx(inputClass, "num h-7.5 w-16 text-right")}
                      />
                    </td>
                    <td className="num px-3 py-2 text-right text-muted">{p ? money(p.lowestSell, currency) : "—"}</td>
                    <td className="num px-3 py-2 text-right text-muted">{p ? money(p.highestBuy, currency) : "—"}</td>
                    <td className="px-3 py-2 text-right">
                      <input
                        aria-label={`Price for ${i.name}`}
                        inputMode="decimal"
                        placeholder={p?.suggested ? (p.suggested / 100).toFixed(2) : "0.00"}
                        value={price[i.marketHashName] ?? ""}
                        onChange={(e) => setPrice((x) => ({ ...x, [i.marketHashName]: e.target.value }))}
                        className={cx(inputClass, "num h-7.5 w-20 text-right")}
                      />
                    </td>
                    <td className="num px-4 py-2 text-right">{each ? money(sellerReceives(each, wallet), currency) : "—"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {unmarketable > 0 && (
        <div className="mt-3 text-[12px] text-faint">
          {unmarketable} item type(s) hidden because Steam doesn't allow them on the market (gems, recently acquired items).
        </div>
      )}

      {sellable.length > 0 && (
        <div className="sticky bottom-4 mt-4 flex items-center gap-4 rounded-xl border border-line-strong bg-panel px-4 py-3 shadow-2xl">
          <span className="text-[13px]">
            <span className="num">{sellable.reduce((n, i) => n + qtyOf(i), 0)}</span> item(s), buyers pay{" "}
            <span className="num">{money(totalBuyer, currency)}</span>, you receive{" "}
            <span className="num text-ok">{money(totalYou, currency)}</span>
          </span>
          <Button className="ml-auto" variant="primary" onClick={() => setConfirm(true)}>
            List on the market
          </Button>
        </div>
      )}

      {confirm && (
        <Modal title="List these items?" onClose={() => setConfirm(false)}>
          <div className="flex flex-col gap-4">
            <p className="text-muted">
              IdleKit will create <span className="num text-ink">{sellable.reduce((n, i) => n + qtyOf(i), 0)}</span> listings,
              pausing {settings.market.delayMs} ms between each to stay under Steam's limits. If your account uses the
              Steam Mobile Authenticator, each listing waits for you to confirm it in the app.
            </p>
            <div className="flex justify-between rounded-lg bg-well px-4 py-3">
              <span className="text-muted">You receive after Steam's fees</span>
              <span className="num text-[17px] text-ok">{money(totalYou, currency)}</span>
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setConfirm(false)}>
                Cancel
              </Button>
              <Button
                variant="primary"
                busy={busy === "sell"}
                onClick={async () => {
                  setBusy("sell");
                  const req = sellable.map((i) => ({
                    marketHashName: i.marketHashName,
                    assetIds: i.assetIds.slice(0, qtyOf(i)),
                    buyerCents: priceOf(i)!,
                  }));
                  const r = await run(null, () => api.sell(account.steamId, req));
                  setBusy(null);
                  setConfirm(false);
                  if (r) {
                    const ok = r.filter((x) => x.ok).length;
                    const conf = r.filter((x) => x.needsConfirmation).length;
                    useApp
                      .getState()
                      .toast(
                        `Listed ${ok} of ${r.length}${conf ? `; confirm ${conf} in the Steam Mobile app` : ""}`,
                        ok === r.length ? "ok" : "error",
                      );
                    // What was listed leaves the inventory and joins the listings.
                    void load();
                    void useMarketStore.getState().loadListings(account.steamId);
                  }
                }}
              >
                List {sellable.reduce((n, i) => n + qtyOf(i), 0)} items
              </Button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}

function Listings({ account }: { account: Account }) {
  const run = useApp((s) => s.run);
  const { listings: all, listingsLoading: loading } = useMarket(account.steamId);
  // A listing only names its game ("<game> Trading Card"), so "Hide NSFW
  // games" goes by the names of the games in the library that it hides.
  useEnsureGames(account.steamId);
  const games = useApp((s) => s.games[account.steamId]);
  const hiding = useHiding(useAppIds(games));
  const list = useMemo(() => {
    if (!all || !hiding.on) return all;
    // Until the library is known, there is no telling which listings to hide.
    if (!games) return [];
    const names = games.filter((g) => hiding.hides(g.appid) && g.name.trim()).map((g) => g.name.trim().toLowerCase());
    return all.filter((l) => !names.some((name) => l.game.toLowerCase().startsWith(name)));
  }, [all, games, hiding]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);

  const load = () => useMarketStore.getState().loadListings(account.steamId);
  // Fresh listings start with nothing selected.
  useEffect(() => setSelected(new Set()), [all]);

  if (!list) {
    return loading ? (
      <Empty title="Loading your listings…" body="What you have for sale, and anything still waiting for mobile confirmation." />
    ) : (
      <Empty
        title="Load your listings"
        body="Shows what you have for sale and anything still waiting for mobile confirmation."
        action={
          <Button variant="primary" onClick={() => void load()}>
            Load listings
          </Button>
        }
      />
    );
  }

  return (
    <div>
      <ProgressBar />
      <div className="mb-3 flex items-center gap-2">
        <Button size="sm" variant="ghost" onClick={() => setSelected(new Set(list.map((l) => l.listingId)))}>
          Select all
        </Button>
        <div className="ml-auto flex gap-2">
          <Button size="sm" icon={<RefreshCw size={13} className={cx(loading && "animate-spin")} />} disabled={loading} onClick={() => void load()}>
            Refresh
          </Button>
          <Button
            size="sm"
            variant="danger"
            icon={<Trash size={13} />}
            disabled={selected.size === 0}
            busy={busy}
            onClick={async () => {
              setBusy(true);
              const n = await run(null, () => api.removeListings(account.steamId, [...selected]));
              setBusy(false);
              if (n !== undefined) {
                useApp.getState().toast(`Removed ${n} listing(s)`, "ok");
                void load();
              }
            }}
          >
            Remove {selected.size || ""} selected
          </Button>
        </div>
      </div>
      <HiddenNotice hiding={hiding} />
      {list.length === 0 ? (
        <Empty title="Nothing listed" body="Items you list from the Inventory tab show up here." />
      ) : (
        <div className="overflow-hidden rounded-xl border border-line bg-panel">
          {list.map((l, idx) => (
            <label key={l.listingId} className={cx("flex cursor-pointer items-center gap-4 px-4 py-2.5", idx > 0 && "border-t border-line")}>
              <input
                type="checkbox"
                className="size-4 accent-[var(--ink)]"
                checked={selected.has(l.listingId)}
                onChange={() =>
                  setSelected((s) => {
                    const n = new Set(s);
                    if (n.has(l.listingId)) n.delete(l.listingId);
                    else n.add(l.listingId);
                    return n;
                  })
                }
              />
              <div className="min-w-0 flex-1">
                <div className="truncate">{l.name}</div>
                <div className="truncate text-[12px] text-faint">{l.game}</div>
              </div>
              <span className="num text-muted">{l.priceText}</span>
              <Pill tone={l.status === "pending" ? "run" : "neutral"}>
                {l.status === "pending" ? "Awaiting confirmation" : "Listed"}
              </Pill>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}
