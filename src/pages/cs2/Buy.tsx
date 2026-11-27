// Buying with the Steam Wallet: one listing at the price shown, or an order
// to buy at up to a price. Nothing is sent before the buyer has seen the
// total, agreed to Steam's terms and pressed the button; what Steam asks to
// be confirmed in the mobile app is waited for, and can be given up on.
import { useEffect, useRef, useState } from "react";
import { Button, Callout, Modal, inputClass } from "../../components/ui";
import { api } from "../../lib/api";
import { typedAmount, useCs2, useCs2Store } from "../../lib/cs2";
import { currencyCode, money, sellerReceives } from "../../lib/format";
import type { Cs2Bought, Cs2Buyer, Cs2Depth, Cs2Listing, Cs2OrderStatus } from "../../lib/types";
import { Agree, ItemIcon, ItemName, Line, Wear } from "./parts";

/** How often Steam is asked whether a purchase was confirmed, and for how long. */
const CONFIRM_EVERY_MS = 3000;
const CONFIRM_TRIES = 40;

type Phase =
  | { at: "review" }
  | { at: "sending" }
  | { at: "confirming"; tries: number }
  | { at: "done"; result: Cs2Bought }
  | { at: "refused"; why: string };

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Sends a purchase once, then, when Steam wants it confirmed, asks again every few seconds until it is, it is refused or the wait is given up. */
function usePurchase(send: (confirmation: string | null) => Promise<Cs2Bought>) {
  const [phase, setPhase] = useState<Phase>({ at: "review" });
  const alive = useRef(true);
  const gaveUp = useRef(false);
  // Set on every mount: a development build mounts everything twice.
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const start = async () => {
    gaveUp.current = false;
    setPhase({ at: "sending" });
    try {
      let r = await send(null);
      for (let tries = 1; r.needsConfirmation && r.confirmationId; tries++) {
        if (!alive.current || gaveUp.current) return;
        if (tries > CONFIRM_TRIES) {
          setPhase({ at: "refused", why: "It wasn't confirmed in time, so nothing was bought. Start again when you have the Steam mobile app at hand." });
          return;
        }
        setPhase({ at: "confirming", tries });
        await wait(CONFIRM_EVERY_MS);
        if (!alive.current || gaveUp.current) return;
        r = await send(r.confirmationId);
      }
      if (!alive.current) return;
      setPhase(r.ok ? { at: "done", result: r } : { at: "refused", why: r.message || "Steam refused without saying why." });
    } catch (e) {
      if (alive.current) setPhase({ at: "refused", why: String(e) });
    }
  };
  const giveUp = () => {
    gaveUp.current = true;
    setPhase({ at: "refused", why: "Stopped waiting, so nothing was bought. Decline the request in the Steam mobile app if it is still there." });
  };
  return { phase, start, giveUp, back: () => setPhase({ at: "review" }) };
}

/** The tax Steam adds on the fees in some countries, worked out as its purchase dialog does. */
function taxOn(fee: number, buyer: Cs2Buyer | null): number {
  return buyer && buyer.taxPercent > 0 && fee > 2 ? Math.floor((fee * buyer.taxPercent) / 100 + 0.5) : 0;
}

/** What keeps a purchase of `total` in `currency` from being sent, in words; null when nothing does. */
function objection(buyer: Cs2Buyer | null, currency: number, total: number): string | null {
  if (!buyer) return null;
  if (currency !== buyer.currency) return "Steam shows this price in another currency than your wallet's, so it can't be bought from here. Open the item again.";
  if (buyer.balance !== null && total > buyer.balance) return `Your wallet has ${money(buyer.balance, buyer.currency)}: not enough for this.`;
  if (buyer.needsBilling && !buyer.hasBilling)
    return "Steam asks for a billing address with purchases from your country and has none saved. Buy once on Steam's own market page, which saves it.";
  return null;
}

function Waiting({ buyer, tries, onGiveUp }: { buyer: Cs2Buyer | null; tries: number; onGiveUp: () => void }) {
  return (
    <div>
      <Callout tone="run">
        <div className="font-medium">{buyer?.confirmation === 1 ? "Confirm this purchase by e-mail" : "Confirm this purchase in the Steam mobile app"}</div>
        <div className="mt-1 text-muted">
          Steam is waiting for you to approve it. Keep this open: IdleKit asks Steam every few seconds and finishes the purchase as soon as it is
          approved (asked {tries} time{tries === 1 ? "" : "s"} so far).
        </div>
      </Callout>
      <div className="mt-4 flex justify-end">
        <Button onClick={onGiveUp}>Stop waiting</Button>
      </div>
    </div>
  );
}

export function BuyListingDialog({
  steamId,
  listing,
  holdDays,
  onClose,
  onBought,
}: {
  steamId: string;
  listing: Cs2Listing;
  holdDays: number;
  onClose: () => void;
  onBought: () => void;
}) {
  const buyer = useCs2(steamId).buyer;
  const [agreed, setAgreed] = useState(false);
  const [asked, setAsked] = useState(false);
  useEffect(() => {
    void useCs2Store.getState().loadBuyer(steamId).then(() => setAsked(true));
  }, [steamId]);
  const tax = taxOn(listing.fee, buyer);
  const total = listing.price + listing.fee + tax;
  const { phase, start, giveUp, back } = usePurchase((confirmation) =>
    api.cs2BuyListing(steamId, { listingId: listing.listingId, hashName: listing.hashName, price: listing.price, fee: listing.fee, currency: listing.currency }, confirmation),
  );
  useEffect(() => {
    if (phase.at !== "done") return;
    const store = useCs2Store.getState();
    if (phase.result.balance !== null) store.setBuyerBalance(steamId, phase.result.balance);
    else void store.loadBuyer(steamId, true);
    onBought();
    // Only when the purchase is done.
  }, [phase.at]);
  const no = objection(buyer, listing.currency, total);
  const busy = phase.at === "sending" || phase.at === "confirming";
  return (
    <Modal title="Buy this item" onClose={busy ? () => undefined : onClose} width={480}>
      <div className="flex items-center gap-3">
        <ItemIcon src={listing.image ?? listing.iconUrl} size={64} />
        <div className="min-w-0">
          <div className="flex text-[15px] font-medium">
            <ItemName name={listing.name} color={listing.color} />
          </div>
          <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px] text-muted">
            {listing.float !== null && <Wear float={listing.float} />}
            {listing.pattern !== null && <span>Pattern {listing.pattern}</span>}
            {listing.stickers.length > 0 && <span>{listing.stickers.length} sticker(s) or charm(s)</span>}
          </div>
        </div>
      </div>

      {phase.at === "done" ? (
        <div className="mt-4">
          <Callout tone="ok">
            <div className="font-medium">Bought for {money(total, listing.currency)}</div>
            <div className="mt-1 text-muted">
              It is in your CS2 inventory.{holdDays > 0 ? ` Steam keeps a bought item from being traded or sold on for ${holdDays} days.` : ""}
              {phase.result.balance !== null ? ` Your wallet now has ${money(phase.result.balance, listing.currency)}.` : ""}
            </div>
          </Callout>
          <div className="mt-4 flex justify-end">
            <Button variant="primary" onClick={onClose}>
              Done
            </Button>
          </div>
        </div>
      ) : phase.at === "confirming" ? (
        <div className="mt-4">
          <Waiting buyer={buyer} tries={phase.tries} onGiveUp={giveUp} />
        </div>
      ) : (
        <>
          <div className="mt-4 rounded-lg border border-line bg-well/40 px-4 py-2 text-[13px]">
            <Line label="Price (goes to the seller)" value={money(listing.price, listing.currency)} />
            <Line label="Steam's and the game's fees" value={money(listing.fee, listing.currency)} />
            {tax > 0 && <Line label={`Tax on the fees (${buyer?.taxPercent}%)`} value={money(tax, listing.currency)} />}
            <Line strong label="You pay" value={money(total, listing.currency)} />
          </div>
          <div className="mt-2 text-[12px] text-muted">
            {buyer ? (
              buyer.balance !== null ? (
                <>
                  Wallet: <span className="num">{money(buyer.balance, buyer.currency)}</span>
                  {buyer.balance >= total && (
                    <>
                      , <span className="num">{money(buyer.balance - total, buyer.currency)}</span> after this purchase
                    </>
                  )}
                </>
              ) : (
                "Steam didn't say what is in the wallet."
              )
            ) : asked ? (
              "The wallet couldn't be read, so this can't be bought right now."
            ) : (
              "Reading the wallet…"
            )}
          </div>
          {phase.at === "refused" && (
            <div className="mt-3">
              <Callout tone="bad">
                <div className="font-medium">Not bought</div>
                <div className="mt-1">{phase.why}</div>
              </Callout>
            </div>
          )}
          {no && phase.at !== "refused" && (
            <div className="mt-3">
              <Callout tone="bad">{no}</Callout>
            </div>
          )}
          <div className="mt-4">
            <Agree checked={agreed} onChange={setAgreed} />
          </div>
          <div className="mt-4 flex justify-end gap-2">
            <Button onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            {phase.at === "refused" ? (
              <Button onClick={back}>Back</Button>
            ) : (
              <Button variant="primary" busy={phase.at === "sending"} disabled={!buyer || !agreed || !!no} onClick={() => void start()}>
                Buy for {money(total, listing.currency)}
              </Button>
            )}
          </div>
        </>
      )}
    </Modal>
  );
}

/** How often and how long a new order is looked at to see whether it bought at once. */
const STATUS_EVERY_MS = 2500;
const STATUS_TRIES = 5;

export function BuyOrderDialog({
  steamId,
  hashName,
  name,
  iconUrl,
  depth,
  buyNow,
  onClose,
  onPlaced,
}: {
  steamId: string;
  hashName: string;
  name: string;
  iconUrl: string;
  depth: Cs2Depth | null;
  /** Opened to buy at the cheapest price there is, not to wait for a lower one. */
  buyNow: boolean;
  onClose: () => void;
  onPlaced: () => void;
}) {
  const buyer = useCs2(steamId).buyer;
  const [asked, setAsked] = useState(false);
  useEffect(() => {
    void useCs2Store.getState().loadBuyer(steamId).then(() => setAsked(true));
  }, [steamId]);
  const currency = buyer?.currency ?? depth?.currency ?? 1;
  const step = buyer?.wallet.increment ?? 1;
  const start0 = buyNow ? depth?.lowestSell : depth?.highestBuy != null ? depth.highestBuy + step : depth?.lowestSell;
  const [price, setPrice] = useState(start0 ? (start0 / 100).toFixed(2) : "");
  const [quantity, setQuantity] = useState("1");
  const [agreed, setAgreed] = useState(false);
  const [status, setStatus] = useState<Cs2OrderStatus | null>(null);
  const each = typedAmount(price);
  const count = /^\d+$/.test(quantity.trim()) ? Number(quantity) : 0;
  const total = each !== null ? each * count : 0;
  const { phase, start, giveUp, back } = usePurchase((confirmation) => api.cs2BuyOrder(steamId, hashName, each ?? 0, count, confirmation));
  const alive = useRef(true);
  // Set on every mount: a development build mounts everything twice.
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  // A new order is looked at a few times: one at or above the cheapest listing buys within moments.
  useEffect(() => {
    if (phase.at !== "done") return;
    onPlaced();
    const id = phase.result.orderId;
    if (!id) return;
    void (async () => {
      for (let i = 0; i < STATUS_TRIES && alive.current; i++) {
        await wait(STATUS_EVERY_MS);
        if (!alive.current) return;
        try {
          const s = await api.cs2OrderStatus(steamId, id);
          setStatus(s);
          if (!s.active || s.remaining === 0) break;
        } catch {
          break;
        }
      }
      if (alive.current) {
        void useCs2Store.getState().loadBuyer(steamId, true);
        onPlaced();
      }
    })();
    // Only when the order is placed.
  }, [phase.at]);

  const lowest = depth?.lowestSell ?? null;
  const tooLow = each !== null && buyer ? sellerReceives(each, buyer.wallet) === 0 : false;
  const badInput =
    each === null
      ? "Type the most you would pay for one."
      : count < 1 || count > 100
        ? "An order from here is for 1 to 100 of an item."
        : tooLow
          ? "That is below the lowest price an item can sell at."
          : null;
  // An order may stand for more than the wallet holds, up to ten times it; buying at once needs the money there.
  const overLimit = buyer?.balance != null && total > buyer.balance * 10;
  const no =
    objection(buyer ? { ...buyer, balance: null } : null, currency, total) ??
    (overLimit ? `Steam lets your buy orders add up to ten times your wallet (${money((buyer?.balance ?? 0) * 10, currency)}).` : null);
  const buysAtOnce = each !== null && lowest !== null && each >= lowest;
  const short = buysAtOnce && buyer?.balance != null && total > buyer.balance;
  const busy = phase.at === "sending" || phase.at === "confirming";

  return (
    <Modal title={buyNow ? "Buy" : "Place a buy order"} onClose={busy ? () => undefined : onClose} width={480}>
      <div className="flex items-center gap-3">
        <ItemIcon src={iconUrl} size={56} />
        <div className="min-w-0">
          <div className="truncate text-[15px] font-medium">{name}</div>
          <div className="truncate text-[12px] text-muted">{hashName}</div>
        </div>
      </div>

      {phase.at === "done" ? (
        <div className="mt-4">
          {status && status.bought > 0 ? (
            <Callout tone="ok">
              <div className="font-medium">
                Bought {status.bought} of {count}
                {status.paidText ? ` for ${status.paidText}` : ""}
              </div>
              <div className="mt-1 text-muted">
                {status.remaining > 0 && status.active ? `The order stays open for the other ${status.remaining}. ` : ""}What you bought is in your CS2 inventory.
              </div>
            </Callout>
          ) : (
            <Callout tone="ok">
              <div className="font-medium">The order is placed</div>
              <div className="mt-1 text-muted">
                {status
                  ? "Nothing sells at your price right now. The order stays open and buys as soon as someone lists at or below it; it is under My orders, where it can be cancelled."
                  : "Looking whether it buys right away…"}
              </div>
            </Callout>
          )}
          <div className="mt-4 flex justify-end">
            <Button variant="primary" onClick={onClose}>
              Done
            </Button>
          </div>
        </div>
      ) : phase.at === "confirming" ? (
        <div className="mt-4">
          <Waiting buyer={buyer} tries={phase.tries} onGiveUp={giveUp} />
        </div>
      ) : (
        <>
          <div className="mt-4 grid grid-cols-2 gap-3">
            <label className="text-[12px] text-muted">
              Most you pay for one, in {currencyCode(currency)}
              <input
                className={`${inputClass} num mt-1 w-full`}
                inputMode="decimal"
                value={price}
                disabled={busy}
                onChange={(e) => setPrice(e.target.value)}
                aria-label="Most you pay for one"
              />
            </label>
            <label className="text-[12px] text-muted">
              How many
              <input
                className={`${inputClass} num mt-1 w-full`}
                inputMode="numeric"
                value={quantity}
                disabled={busy}
                onChange={(e) => setQuantity(e.target.value)}
                aria-label="How many"
              />
            </label>
          </div>
          <div className="mt-2 text-[12px] text-muted">
            {depth ? (
              <>
                Cheapest for sale: <span className="num">{money(depth.lowestSell, depth.currency)}</span>. Highest offer so far:{" "}
                <span className="num">{money(depth.highestBuy, depth.currency)}</span>. The price you offer includes Steam's fees.
              </>
            ) : (
              "The price you offer includes Steam's fees."
            )}
          </div>
          <div className="mt-3 rounded-lg border border-line bg-well/40 px-4 py-2 text-[13px]">
            <Line strong label={count > 1 ? `Up to, for ${count}` : "Up to"} value={each !== null && count > 0 ? money(total, currency) : "—"} />
          </div>
          <div className="mt-2 text-[12px] text-muted">
            {badInput ??
              (buysAtOnce
                ? "At this price the order buys right away, at the cheapest price there is, and gives back the difference."
                : "At this price the order waits until someone sells for it. It can be cancelled any time under My orders.")}{" "}
            {buyer ? (
              buyer.balance !== null ? (
                <>
                  Wallet: <span className="num">{money(buyer.balance, buyer.currency)}</span>.
                </>
              ) : null
            ) : asked ? (
              "The wallet couldn't be read, so no order can be placed right now."
            ) : (
              "Reading the wallet…"
            )}
          </div>
          {short && !no && (
            <div className="mt-3">
              <Callout tone="run">Your wallet has less than this order would cost, so it can only buy once there is enough in it.</Callout>
            </div>
          )}
          {phase.at === "refused" && (
            <div className="mt-3">
              <Callout tone="bad">
                <div className="font-medium">Not placed</div>
                <div className="mt-1">{phase.why}</div>
              </Callout>
            </div>
          )}
          {no && phase.at !== "refused" && (
            <div className="mt-3">
              <Callout tone="bad">{no}</Callout>
            </div>
          )}
          <div className="mt-4">
            <Agree checked={agreed} onChange={setAgreed} />
          </div>
          <div className="mt-4 flex justify-end gap-2">
            <Button onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            {phase.at === "refused" ? (
              <Button onClick={back}>Back</Button>
            ) : (
              <Button variant="primary" busy={phase.at === "sending"} disabled={!buyer || !agreed || !!badInput || !!no} onClick={() => void start()}>
                {buysAtOnce ? `Buy ${count > 1 ? `${count} ` : ""}for up to ${money(total, currency)}` : "Place the order"}
              </Button>
            )}
          </div>
        </>
      )}
    </Modal>
  );
}
