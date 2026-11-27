// Selling from the CS2 inventory: every item with its wear, what the
// market asks and offers for it once it is ticked, a price to ask with what
// arrives after Steam's fees, and one last look before anything is listed.
import { RefreshCw, Search, Tag } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button, Callout, Empty, Modal, Pill, TextInput, cx, smallInputClass } from "../../components/ui";
import { api } from "../../lib/api";
import { typedAmount, useCs2, useCs2Store } from "../../lib/cs2";
import { US_DOLLAR_WALLET, money, plural, sellerReceives, suggestPrice } from "../../lib/format";
import { useKept } from "../../lib/kept";
import { useApp } from "../../lib/store";
import type { Account, Cs2Depth, Cs2Item } from "../../lib/types";
import { ItemIcon, ItemName, Line, Wear } from "./parts";

/** Items that are the same in every way that sells (cases, keys, stickers) share a row; a skin has a row of its own. */
interface Row {
  key: string;
  item: Cs2Item;
  /** The ones of it that can be sold. */
  sellable: Cs2Item[];
  count: number;
}

function rowsOf(items: Cs2Item[]): Row[] {
  const rows = new Map<string, Row>();
  for (const item of items) {
    const alike = item.float === null && item.pattern === null && !item.protected && item.hashName !== "";
    const key = alike ? `n:${item.hashName}:${item.marketable ? 1 : 0}` : `a:${item.assetId}`;
    const row = rows.get(key) ?? { key, item, sellable: [], count: 0 };
    row.count += 1;
    if (item.marketable) row.sellable.push(item);
    rows.set(key, row);
  }
  return [...rows.values()];
}

/** What an item's order book said, or that it is being read, or why it couldn't be. */
type Looked = Cs2Depth | "reading" | "none" | { error: string };

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
      <span className="num text-[12px] text-muted">
        {p.done} / {p.total}
      </span>
      <Button size="sm" onClick={() => void api.marketCancel()}>
        Cancel
      </Button>
    </div>
  );
}

export function SellTab({ account }: { account: Account }) {
  const steamId = account.steamId;
  const { inventory, inventoryLoading, buyer } = useCs2(steamId);
  const settings = useApp((s) => s.settings);
  const [query, setQuery] = useKept("cs2.sell.query", "");
  const [onlySellable, setOnlySellable] = useKept("cs2.sell.only", true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [books, setBooks] = useKept<Record<string, Looked>>(`cs2.sell.books.${steamId}`, {});
  const [typed, setTyped] = useState<Record<string, string>>({});
  const [amounts, setAmounts] = useState<Record<string, string>>({});
  const [review, setReview] = useState(false);
  const [busy, setBusy] = useState(false);
  const wallet = buyer?.wallet ?? US_DOLLAR_WALLET;

  useEffect(() => {
    const store = useCs2Store.getState();
    if (!inventory && !inventoryLoading) void store.loadInventory(steamId);
    // The wallet's currency and fee numbers, for "you receive".
    if (!buyer) void store.loadBuyer(steamId, true);
    // Once per account, when the tab opens.
  }, [steamId]);

  const rows = useMemo(() => rowsOf(inventory ?? []), [inventory]);
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return rows.filter((r) => (!onlySellable || r.sellable.length > 0) && (!q || r.item.name.toLowerCase().includes(q) || r.item.kind.toLowerCase().includes(q)));
  }, [rows, query, onlySellable]);
  // What is ticked and still there to sell.
  const ticked = useMemo(() => rows.filter((r) => selected.has(r.key) && r.sellable.length > 0), [rows, selected]);

  // The order books of ticked items are read one after the other, at the market's pace.
  const reading = useRef(false);
  const wanted = useMemo(() => [...new Set(ticked.map((r) => r.item.hashName))].filter((name) => name && books[name] === undefined), [ticked, books]);
  useEffect(() => {
    if (reading.current || wanted.length === 0) return;
    reading.current = true;
    const name = wanted[0];
    setBooks((b) => ({ ...b, [name]: "reading" }));
    void (async () => {
      let looked: Looked;
      try {
        looked = (await api.cs2Book(steamId, name)) ?? "none";
      } catch (e) {
        looked = { error: String(e) };
      }
      await new Promise((r) => setTimeout(r, Math.max(300, settings?.market.delayMs ?? 1500)));
      reading.current = false;
      setBooks((b) => ({ ...b, [name]: looked }));
    })();
    // The next one starts when this one's answer is kept.
  }, [wanted]);

  const depthOf = (r: Row): Cs2Depth | null => {
    const b = books[r.item.hashName];
    return b && typeof b === "object" && "currency" in b ? b : null;
  };
  /** What the buyer pays for one of a row: what was typed, else a price by the market strategy in Settings. */
  const priceOf = (r: Row): number | null => {
    const own = typed[r.key];
    if (own !== undefined && own.trim() !== "") return typedAmount(own);
    const d = depthOf(r);
    if (!d || !settings || d.currency !== wallet.currency) return null;
    return suggestPrice(wallet, d.lowestSell, d.highestBuy, settings.market);
  };
  const amountOf = (r: Row) => {
    const n = Number(amounts[r.key] ?? "1");
    return Number.isInteger(n) ? Math.min(Math.max(n, 1), r.sellable.length) : 1;
  };
  const toList = ticked
    .map((r) => ({ row: r, price: priceOf(r), amount: amountOf(r) }))
    .filter((x): x is { row: Row; price: number; amount: number } => x.price !== null && sellerReceives(x.price, wallet) > 0);
  const listings = toList.reduce((n, x) => n + x.amount, 0);
  const receive = toList.reduce((n, x) => n + sellerReceives(x.price, wallet) * x.amount, 0);

  const toggle = (key: string) =>
    setSelected((s) => {
      const next = new Set(s);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  const sell = async () => {
    setBusy(true);
    const items = toList.flatMap((x) => x.row.sellable.slice(0, x.amount).map((a) => ({ assetId: a.assetId, hashName: a.hashName, buyerCents: x.price })));
    const out = await useApp.getState().run(null, () => api.cs2Sell(steamId, items));
    setBusy(false);
    setReview(false);
    if (!out) return;
    const ok = out.filter((o) => o.ok).length;
    const confirm = out.filter((o) => o.ok && o.needsConfirmation).length;
    const failed = out.find((o) => !o.ok);
    useApp
      .getState()
      .toast(
        `Listed ${ok} of ${out.length}${confirm ? `; ${confirm} to confirm in the Steam mobile app` : ""}${failed ? `. ${failed.message}` : ""}`,
        failed ? (ok ? "info" : "error") : "ok",
      );
    setSelected(new Set());
    setTyped({});
    const store = useCs2Store.getState();
    void store.loadInventory(steamId);
    void store.loadMine(steamId);
  };

  if (!inventory) {
    return inventoryLoading ? (
      <Empty title="Loading your CS2 inventory…" body="Every Counter-Strike 2 item on this account, with what can be sold." />
    ) : (
      <Empty
        title="Load your CS2 inventory"
        body="Shows your Counter-Strike 2 items and what each can be listed for."
        action={
          <Button variant="primary" onClick={() => void useCs2Store.getState().loadInventory(steamId)}>
            Load inventory
          </Button>
        }
      />
    );
  }

  const protectedCount = inventory.filter((i) => i.protected).length;
  return (
    <div>
      <ProgressBar />
      <div className="mb-3 flex flex-wrap items-center gap-3">
        <div className="relative min-w-[220px] flex-1">
          <Search size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-faint" />
          <TextInput className="w-full pl-8" placeholder="Filter your items" aria-label="Filter your items" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <label className="flex cursor-pointer items-center gap-2 text-[13px] text-muted">
          <input type="checkbox" className="size-4 accent-[var(--ink)]" checked={onlySellable} onChange={(e) => setOnlySellable(e.target.checked)} />
          Only what can be sold
        </label>
        <Button size="sm" icon={<RefreshCw size={13} className={cx(inventoryLoading && "animate-spin")} />} disabled={inventoryLoading} onClick={() => void useCs2Store.getState().loadInventory(steamId)}>
          Refresh
        </Button>
        <Button variant="primary" icon={<Tag size={14} />} disabled={toList.length === 0} onClick={() => setReview(true)}>
          List {listings || ""} selected…
        </Button>
      </div>
      <div className="mb-3 text-[12px] text-faint">
        {plural(inventory.length, "item")}, {inventory.filter((i) => i.marketable).length} to sell
        {protectedCount > 0 ? `, ${protectedCount} trade protected (received in a trade in the last week)` : ""}. Tick an item to see what the market asks and offers for it.
      </div>

      {shown.length === 0 ? (
        <Empty title="Nothing here" body={inventory.length === 0 ? "This account has no Counter-Strike 2 items, or its inventory can't be read." : "No item matches this filter."} />
      ) : (
        <div className="overflow-x-auto rounded-xl border border-line bg-panel">
          <table className="w-full text-left text-[13px]">
            <thead className="border-b border-line text-[12px] text-faint">
              <tr>
                <th className="w-10 px-4 py-2.5" />
                <th className="py-2.5 font-normal">Item</th>
                <th className="px-3 py-2.5 font-normal">Wear</th>
                <th className="px-3 py-2.5 text-right font-normal">Lowest listing</th>
                <th className="px-3 py-2.5 text-right font-normal">Highest offer</th>
                <th className="px-3 py-2.5 text-right font-normal">Sell</th>
                <th className="px-3 py-2.5 text-right font-normal">Buyer pays</th>
                <th className="px-4 py-2.5 text-right font-normal">You receive</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => {
                const sel = selected.has(r.key);
                const can = r.sellable.length > 0;
                const looked = books[r.item.hashName];
                const d = depthOf(r);
                const price = sel ? priceOf(r) : null;
                const none = looked === "reading" ? "…" : "—";
                const about =
                  looked === undefined
                    ? undefined
                    : looked === "reading"
                      ? "Its order book is on the way"
                      : looked === "none"
                        ? "Steam has no market for this item"
                        : "error" in looked
                          ? looked.error
                          : undefined;
                return (
                  <tr key={r.key} className={cx("border-t border-line first:border-t-0", sel && "bg-raised/50", !can && "opacity-70")}>
                    <td className="px-4 py-2">
                      <input type="checkbox" className="size-4 accent-[var(--ink)]" checked={sel} disabled={!can} onChange={() => toggle(r.key)} aria-label={`Select ${r.item.name}`} />
                    </td>
                    <td className="py-2">
                      <div className="flex items-center gap-3">
                        <ItemIcon src={r.item.iconUrl} size={44} />
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <ItemName name={r.item.name} color={r.item.color} />
                            {r.count > 1 && <span className="num flex-none text-[12px] text-faint">× {r.count}</span>}
                            {r.item.protected && <Pill tone="run">Trade protected</Pill>}
                            {!r.item.protected && !can && <Pill>Can't be sold</Pill>}
                          </div>
                          <div className="truncate text-[12px] text-faint">
                            {r.item.kind}
                            {r.item.notes.length > 0 ? ` · ${r.item.notes[0]}` : ""}
                          </div>
                        </div>
                      </div>
                    </td>
                    <td className="px-3 py-2">{r.item.float !== null ? <Wear float={r.item.float} /> : <span className="text-faint">{r.item.exterior || "—"}</span>}</td>
                    <td className="num px-3 py-2 text-right text-muted" title={about}>
                      {d ? money(d.lowestSell, d.currency) : none}
                    </td>
                    <td className="num px-3 py-2 text-right text-muted" title={about}>
                      {d ? money(d.highestBuy, d.currency) : none}
                    </td>
                    <td className="px-3 py-2 text-right">
                      {r.sellable.length > 1 ? (
                        <input
                          aria-label={`How many ${r.item.name} to sell`}
                          type="number"
                          min={1}
                          max={r.sellable.length}
                          disabled={!sel}
                          value={amounts[r.key] ?? "1"}
                          onChange={(e) => setAmounts((a) => ({ ...a, [r.key]: e.target.value }))}
                          className={cx(smallInputClass, "num w-16 text-right")}
                        />
                      ) : (
                        <span className="num text-muted">{can ? 1 : "—"}</span>
                      )}
                    </td>
                    <td className="px-3 py-2 text-right">
                      <input
                        aria-label={`Price a buyer pays for ${r.item.name}`}
                        inputMode="decimal"
                        disabled={!sel}
                        placeholder={sel && price !== null ? (price / 100).toFixed(2) : ""}
                        value={typed[r.key] ?? ""}
                        onChange={(e) => setTyped((t) => ({ ...t, [r.key]: e.target.value }))}
                        className={cx(smallInputClass, "num w-24 text-right")}
                      />
                    </td>
                    <td className="num px-4 py-2 text-right">{price !== null ? money(sellerReceives(price, wallet), wallet.currency) : "—"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {review && (
        <Modal title="List these on the market?" onClose={busy ? () => undefined : () => setReview(false)} width={520}>
          <div className="max-h-[300px] overflow-auto rounded-lg border border-line">
            {toList.map((x, i) => (
              <div key={x.row.key} className={cx("flex items-center gap-3 px-3 py-2 text-[13px]", i > 0 && "border-t border-line")}>
                <ItemIcon src={x.row.item.iconUrl} size={32} />
                <div className="min-w-0 flex-1">
                  <div className="flex">
                    <ItemName name={x.row.item.name} color={x.row.item.color} />
                  </div>
                  {x.row.item.float !== null && <div className="text-[12px] text-faint">Wear {x.row.item.float.toFixed(4)}</div>}
                </div>
                <span className="num text-muted">{x.amount > 1 ? `${x.amount} × ` : ""}</span>
                <span className="num">{money(x.price, wallet.currency)}</span>
              </div>
            ))}
          </div>
          <div className="mt-3 rounded-lg border border-line bg-well/40 px-4 py-2 text-[13px]">
            <Line label={`Buyers pay, for ${plural(listings, "listing")}`} value={money(toList.reduce((n, x) => n + x.price * x.amount, 0), wallet.currency)} />
            <Line strong label="You receive, after Steam's fees" value={money(receive, wallet.currency)} />
          </div>
          {ticked.length > toList.length && (
            <div className="mt-3">
              <Callout>{ticked.length - toList.length} ticked item(s) have no price yet and are left out. Type a price for them, or wait for their order book.</Callout>
            </div>
          )}
          <div className="mt-3 text-[12px] text-muted">A listing may have to be confirmed in the Steam mobile app before it shows on the market. It can be taken back under My orders.</div>
          <div className="mt-4 flex justify-end gap-2">
            <Button onClick={() => setReview(false)} disabled={busy}>
              Cancel
            </Button>
            <Button variant="primary" busy={busy} onClick={() => void sell()}>
              List {plural(listings, "item")}
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}
