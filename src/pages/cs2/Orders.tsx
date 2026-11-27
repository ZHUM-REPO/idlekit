// The account's own CS2 listings and buy orders: what is for sale and
// what is waiting to buy, with taking a listing back and cancelling an order.
import { RefreshCw, Trash, X } from "lucide-react";
import { useEffect, useState } from "react";
import { Button, Empty, Pill, cx } from "../../components/ui";
import { api } from "../../lib/api";
import { useCs2, useCs2Store } from "../../lib/cs2";
import { money, plural } from "../../lib/format";
import { useApp } from "../../lib/store";
import type { Account, Cs2MyListing } from "../../lib/types";
import { ItemIcon, ItemName } from "./parts";

const STATUS: Record<Cs2MyListing["status"], { label: string; tone: "neutral" | "run" }> = {
  active: { label: "Listed", tone: "neutral" },
  pending: { label: "Awaiting confirmation", tone: "run" },
  held: { label: "On hold", tone: "run" },
};

export function OrdersTab({ account }: { account: Account }) {
  const steamId = account.steamId;
  const { mine, mineLoading } = useCs2(steamId);
  const run = useApp((s) => s.run);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [removing, setRemoving] = useState(false);
  const [cancelling, setCancelling] = useState<string | null>(null);
  const load = () => useCs2Store.getState().loadMine(steamId);

  useEffect(() => {
    if (!mine && !mineLoading) void load();
    // Once per account, when the tab opens.
  }, [steamId]);
  // Fresh listings start with nothing selected.
  useEffect(() => setSelected(new Set()), [mine]);

  if (!mine) {
    return mineLoading ? (
      <Empty title="Loading your listings and buy orders…" body="What you have for sale on the CS2 market, and what you have orders in for." />
    ) : (
      <Empty
        title="Load your listings and buy orders"
        body="Shows your Counter-Strike 2 items for sale and your open orders to buy."
        action={
          <Button variant="primary" onClick={() => void load()}>
            Load
          </Button>
        }
      />
    );
  }

  const remove = async () => {
    setRemoving(true);
    const n = await run(null, () => api.removeListings(steamId, [...selected]));
    setRemoving(false);
    if (n === undefined) return;
    useApp.getState().toast(`Removed ${n} listing(s)`, "ok");
    void load();
    // What was taken off the market is back in the inventory.
    if (n > 0) void useCs2Store.getState().loadInventory(steamId);
  };
  const cancel = async (orderId: string) => {
    setCancelling(orderId);
    const done = await run(null, async () => {
      await api.cs2CancelOrder(steamId, orderId);
      return true;
    });
    setCancelling(null);
    if (done) {
      useApp.getState().toast("Buy order cancelled", "ok");
      void load();
    }
  };

  return (
    <div>
      <div className="mb-3 flex items-center gap-2">
        <h2 className="text-[15px] font-semibold">For sale</h2>
        <span className="text-[12px] text-faint">{plural(mine.listings.length, "listing")}</span>
        <div className="ml-auto flex gap-2">
          <Button size="sm" icon={<RefreshCw size={13} className={cx(mineLoading && "animate-spin")} />} disabled={mineLoading} onClick={() => void load()}>
            Refresh
          </Button>
          <Button size="sm" variant="danger" icon={<Trash size={13} />} disabled={selected.size === 0} busy={removing} onClick={() => void remove()}>
            Remove {selected.size || ""} selected
          </Button>
        </div>
      </div>
      {mine.listings.length === 0 ? (
        <Empty title="Nothing listed" body="CS2 items you list from the Sell tab show up here." />
      ) : (
        <div className="overflow-hidden rounded-xl border border-line bg-panel">
          {mine.listings.map((l, i) => (
            <label key={l.listingId} className={cx("flex cursor-pointer items-center gap-4 px-4 py-2.5", i > 0 && "border-t border-line")}>
              <input
                type="checkbox"
                className="size-4 accent-[var(--ink)]"
                checked={selected.has(l.listingId)}
                aria-label={`Select the listing of ${l.name}`}
                onChange={() =>
                  setSelected((s) => {
                    const next = new Set(s);
                    if (next.has(l.listingId)) next.delete(l.listingId);
                    else next.add(l.listingId);
                    return next;
                  })
                }
              />
              <ItemIcon src={l.iconUrl} size={40} />
              <div className="flex min-w-0 flex-1">
                <ItemName name={l.name} color={l.color} />
              </div>
              <div className="text-right">
                <div className="num">{money(l.buyerPays, l.currency)}</div>
                <div className="num text-[12px] text-faint">you receive {money(l.youReceive, l.currency)}</div>
              </div>
              <Pill tone={STATUS[l.status].tone}>{STATUS[l.status].label}</Pill>
            </label>
          ))}
        </div>
      )}

      <div className="mb-3 mt-8 flex items-center gap-2">
        <h2 className="text-[15px] font-semibold">Buy orders</h2>
        <span className="text-[12px] text-faint">{plural(mine.orders.length, "order")}</span>
      </div>
      {mine.orders.length === 0 ? (
        <Empty title="No open buy orders" body="An order placed from an item's page waits here until someone sells at its price." />
      ) : (
        <div className="overflow-hidden rounded-xl border border-line bg-panel">
          {mine.orders.map((o, i) => (
            <div key={o.orderId} className={cx("flex items-center gap-4 px-4 py-2.5", i > 0 && "border-t border-line")}>
              <ItemIcon src={o.iconUrl} size={40} />
              <div className="flex min-w-0 flex-1">
                <ItemName name={o.name} color={o.color} />
              </div>
              <div className="text-right">
                <div className="num">
                  {o.remaining.toLocaleString()} × {money(o.price, o.currency)}
                </div>
                <div className="num text-[12px] text-faint">
                  {o.quantity > o.remaining ? `${o.quantity - o.remaining} of ${o.quantity} bought · ` : ""}up to {money(o.price * o.remaining, o.currency)}
                </div>
              </div>
              <Button size="sm" variant="danger" icon={<X size={13} />} busy={cancelling === o.orderId} disabled={cancelling !== null} onClick={() => void cancel(o.orderId)}>
                Cancel
              </Button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
