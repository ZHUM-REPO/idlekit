// Trade offers: the open ones with what each side gives, accepting,
// declining and cancelling them, and making a new offer from the other
// player's trade link. Nothing leaves the account before both sides of the
// trade have been looked at once more.
import { ExternalLink, RefreshCw, Search, Send } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Button, Callout, Empty, Modal, Pill, Tabs, TextInput, cx, inputClass } from "../../components/ui";
import { api, openExternal } from "../../lib/api";
import { useCs2, useCs2Store } from "../../lib/cs2";
import { plural } from "../../lib/format";
import { useKept } from "../../lib/kept";
import { useApp } from "../../lib/store";
import type { Account, Cs2Item, TradeItem, TradeOffer, TradeOutcome } from "../../lib/types";
import { ItemIcon, ItemName, Wear } from "./parts";

type Sub = "received" | "sent" | "new";

/** How the other player is named: by the end of their Steam ID, which the offer is known by. */
function playerName(steamId: string): string {
  return `Player …${steamId.slice(-4)}`;
}

function when(unix: number): string {
  return unix > 0 ? new Date(unix * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "";
}

function Side({ title, items, empty }: { title: string; items: TradeItem[]; empty: string }) {
  return (
    <div className="min-w-0 flex-1">
      <div className="mb-1.5 text-[12px] text-faint">
        {title} ({items.length})
      </div>
      {items.length === 0 ? (
        <div className="rounded-lg border border-dashed border-line px-3 py-3 text-[13px] text-muted">{empty}</div>
      ) : (
        <div className="overflow-hidden rounded-lg border border-line">
          {items.map((it, i) => (
            <div key={`${it.assetId}-${i}`} className={cx("flex items-center gap-2.5 px-3 py-1.5 text-[13px]", i > 0 && "border-t border-line")}>
              <ItemIcon src={it.iconUrl} size={32} />
              <div className="min-w-0 flex-1">
                <div className="flex">
                  <ItemName name={it.name} color={it.color} />
                </div>
                {it.kind && <div className="truncate text-[12px] text-faint">{it.kind}</div>}
              </div>
              {it.missing && <Pill tone="bad">No longer there</Pill>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/** What to look at twice before items change hands. */
function Cautions({ give, receive }: { give: number; receive: number }) {
  return (
    <>
      {give > 0 && receive === 0 && (
        <Callout tone="bad">
          <div className="font-medium">You would give {plural(give, "item")} and get nothing back</div>
          <div className="mt-1">That is a gift. Go on only if you mean to give these away to this player.</div>
        </Callout>
      )}
      <div className="mt-3 text-[12px] text-muted">
        Look at every item on both sides. Nobody from Steam, a trading site or a tournament ever needs your items sent to them "to verify" or "to hold": that is
        the usual way they are stolen.
      </div>
    </>
  );
}

function outcomeText(o: TradeOutcome, done: string): string {
  if (!o.ok) return o.message || "Steam refused without saying why.";
  return o.needsConfirmation ? `${done}. Confirm it ${o.byEmail ? "by e-mail" : "in the Steam mobile app"} to complete it.` : `${done}.`;
}

function OfferCard({ steamId, offer, onChanged }: { steamId: string; offer: TradeOffer; onChanged: () => void }) {
  const run = useApp((s) => s.run);
  const [accepting, setAccepting] = useState(false);
  const [checked, setChecked] = useState(false);
  const [busy, setBusy] = useState<"accept" | "withdraw" | null>(null);
  const [result, setResult] = useState<TradeOutcome | null>(null);
  const active = offer.state === 2;
  const missing = [...offer.give, ...offer.receive].some((i) => i.missing);

  const withdraw = async () => {
    setBusy("withdraw");
    const how = offer.ours ? "cancel" : "decline";
    const done = await run(null, async () => {
      await api.tradeWithdraw(steamId, offer.id, how);
      return true;
    });
    setBusy(null);
    if (done) {
      useApp.getState().toast(offer.ours ? "Offer cancelled" : "Offer declined", "ok");
      onChanged();
    }
  };
  const accept = async () => {
    setBusy("accept");
    try {
      // The offers are read again when this is closed: reading them now
      // would take the answered offer, and what Steam said about it, off the screen.
      setResult(await api.tradeAccept(steamId, offer.id, offer.partner));
    } catch (e) {
      setResult({ ok: false, offerId: null, needsConfirmation: false, byEmail: false, message: String(e) });
    } finally {
      setBusy(null);
    }
  };

  const closeAccepting = () => {
    setAccepting(false);
    if (result?.ok) onChanged();
  };

  return (
    <div className="rounded-xl border border-line bg-panel p-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[14px] font-medium" title={`Steam ID ${offer.partner}`}>
          {offer.ours ? "To" : "From"} {playerName(offer.partner)}
        </span>
        <Button size="sm" variant="ghost" icon={<ExternalLink size={13} />} onClick={() => void openExternal(`https://steamcommunity.com/profiles/${offer.partner}`)}>
          Profile
        </Button>
        <Pill tone={active ? "neutral" : "run"}>{offer.stateText}</Pill>
        {!offer.ours && offer.give.length > 0 && offer.receive.length === 0 && <Pill tone="bad">Asks for items, gives none</Pill>}
        <span className="ml-auto text-[12px] text-faint">
          {when(offer.updated || offer.created)}
          {offer.expires > 0 ? ` · expires ${when(offer.expires)}` : ""}
        </span>
      </div>
      {offer.message && <div className="mt-2 text-[13px] text-muted">"{offer.message}"</div>}
      <div className="mt-3 flex flex-wrap gap-4">
        <Side title="You give" items={offer.give} empty="Nothing" />
        <Side title="You receive" items={offer.receive} empty="Nothing" />
      </div>
      {offer.state === 9 && offer.ours && <div className="mt-3 text-[13px] text-muted">Waiting for you to confirm it in the Steam mobile app.</div>}
      {offer.heldUntil > 0 && <div className="mt-3 text-[13px] text-muted">Steam holds this trade until {when(offer.heldUntil)}.</div>}
      {missing && <div className="mt-3 text-[13px] text-bad">An item of this offer is no longer in its owner's inventory, so it can't be accepted as it is.</div>}
      <div className="mt-3 flex justify-end gap-2">
        <Button size="sm" variant="danger" busy={busy === "withdraw"} disabled={busy !== null} onClick={() => void withdraw()}>
          {offer.ours ? "Cancel offer" : "Decline"}
        </Button>
        {!offer.ours && active && (
          <Button
            size="sm"
            variant="primary"
            disabled={busy !== null || missing}
            onClick={() => {
              setChecked(false);
              setResult(null);
              setAccepting(true);
            }}
          >
            Accept…
          </Button>
        )}
      </div>

      {accepting && (
        <Modal title="Accept this trade?" onClose={busy ? () => undefined : closeAccepting} width={560}>
          <div className="text-[13px] text-muted">With {playerName(offer.partner)}</div>
          <div className="mt-3 flex flex-wrap gap-4">
            <Side title="You give" items={offer.give} empty="Nothing" />
            <Side title="You receive" items={offer.receive} empty="Nothing" />
          </div>
          <div className="mt-3">
            {result ? (
              <Callout tone={result.ok ? "ok" : "bad"}>{outcomeText(result, "Accepted")}</Callout>
            ) : (
              <Cautions give={offer.give.length} receive={offer.receive.length} />
            )}
          </div>
          {!result?.ok && (
            <label className="mt-4 flex cursor-pointer items-start gap-2.5 text-[13px]">
              <input type="checkbox" className="mt-0.5 size-4 accent-[var(--ink)]" checked={checked} onChange={(e) => setChecked(e.target.checked)} />
              <span>I have looked at both sides and want this trade.</span>
            </label>
          )}
          <div className="mt-4 flex justify-end gap-2">
            <Button onClick={closeAccepting} disabled={busy !== null}>
              {result?.ok ? "Close" : "Cancel"}
            </Button>
            {!result?.ok && (
              <Button variant="primary" busy={busy === "accept"} disabled={!checked} onClick={() => void accept()}>
                Accept the trade
              </Button>
            )}
          </div>
        </Modal>
      )}
    </div>
  );
}

/** Items to pick from for one side of a new offer. */
function Picker({
  title,
  items,
  picked,
  onToggle,
  empty,
}: {
  title: string;
  items: Cs2Item[];
  picked: Set<string>;
  onToggle: (assetId: string) => void;
  empty: string;
}) {
  const [query, setQuery] = useState("");
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? items.filter((i) => i.name.toLowerCase().includes(q) || i.kind.toLowerCase().includes(q)) : items;
  }, [items, query]);
  return (
    <div className="min-w-[280px] flex-1 rounded-xl border border-line bg-panel">
      <div className="flex items-center gap-2 border-b border-line px-3 py-2">
        <span className="text-[13px] font-medium">{title}</span>
        <span className="num text-[12px] text-faint">
          {picked.size} of {items.length}
        </span>
        <div className="relative ml-auto w-40">
          <Search size={13} className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-faint" />
          <input className={cx(inputClass, "h-7 w-full pl-7 text-[12px]")} placeholder="Filter" aria-label={`Filter ${title.toLowerCase()}`} value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
      </div>
      {items.length === 0 ? (
        <div className="px-3 py-6 text-[13px] text-muted">{empty}</div>
      ) : (
        <div className="max-h-[340px] overflow-auto">
          {shown.map((it, i) => {
            const on = picked.has(it.assetId);
            return (
              <label key={it.assetId} className={cx("flex cursor-pointer items-center gap-2.5 px-3 py-1.5 text-[13px]", i > 0 && "border-t border-line", on && "bg-raised/60")}>
                <input type="checkbox" className="size-4 accent-[var(--ink)]" checked={on} onChange={() => onToggle(it.assetId)} aria-label={`${on ? "Take out" : "Add"} ${it.name}`} />
                <ItemIcon src={it.iconUrl} size={36} />
                <div className="min-w-0 flex-1">
                  <div className="flex">
                    <ItemName name={it.name} color={it.color} />
                  </div>
                  <div className="flex items-center gap-2 truncate text-[12px] text-faint">
                    {it.float !== null ? <Wear float={it.float} /> : it.kind}
                  </div>
                </div>
              </label>
            );
          })}
          {shown.length === 0 && <div className="px-3 py-4 text-[13px] text-muted">No item matches.</div>}
        </div>
      )}
    </div>
  );
}

function asTradeItems(items: Cs2Item[]): TradeItem[] {
  return items.map((i) => ({ app: 730, context: i.context, assetId: i.assetId, name: i.name, kind: i.kind, iconUrl: i.iconUrl, color: i.color, missing: false }));
}

function NewOffer({ steamId, onSent }: { steamId: string; onSent: () => void }) {
  const { inventory, inventoryLoading } = useCs2(steamId);
  // The link is kept only while IdleKit runs: its token is the other player's to give out.
  const [link, setLink] = useKept(`cs2.trade.link.${steamId}`, "");
  const [partner, setPartner] = useKept<{ link: string; partner: string; items: Cs2Item[] } | null>(`cs2.trade.partner.${steamId}`, null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [give, setGive] = useState<Set<string>>(new Set());
  const [receive, setReceive] = useState<Set<string>>(new Set());
  const [message, setMessage] = useState("");
  const [review, setReview] = useState(false);
  const [checked, setChecked] = useState(false);
  const [sending, setSending] = useState(false);
  const [result, setResult] = useState<TradeOutcome | null>(null);

  useEffect(() => {
    if (!inventory && !inventoryLoading) void useCs2Store.getState().loadInventory(steamId);
    // Once per account, when the tab opens.
  }, [steamId]);

  const mine = useMemo(() => (inventory ?? []).filter((i) => i.tradable), [inventory]);
  const theirs = partner?.items ?? [];
  const giving = mine.filter((i) => give.has(i.assetId));
  const receiving = theirs.filter((i) => receive.has(i.assetId));
  const toggleIn = (set: (f: (s: Set<string>) => Set<string>) => void) => (id: string) =>
    set((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const r = await api.tradePartnerItems(steamId, link.trim());
      setPartner({ link: link.trim(), partner: r.partner, items: r.items });
      setReceive(new Set());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };
  const send = async () => {
    if (!partner) return;
    setSending(true);
    try {
      const ref = (i: Cs2Item) => ({ app: 730, context: i.context, assetId: i.assetId });
      const o = await api.tradeSend(steamId, partner.link, giving.map(ref), receiving.map(ref), message);
      setResult(o);
      if (o.ok) {
        setGive(new Set());
        setReceive(new Set());
        setMessage("");
        onSent();
        // What was offered may no longer be there to offer again.
        void useCs2Store.getState().loadInventory(steamId);
      }
    } catch (e) {
      setResult({ ok: false, offerId: null, needsConfirmation: false, byEmail: false, message: String(e) });
    } finally {
      setSending(false);
    }
  };

  return (
    <div>
      <div className="flex flex-wrap items-end gap-3">
        <label className="min-w-[320px] flex-1 text-[12px] text-muted">
          The other player's trade link, or a friend's 17-digit Steam ID
          <TextInput
            className="mt-1 w-full"
            placeholder="https://steamcommunity.com/tradeoffer/new/?partner=…&token=…"
            value={link}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => setLink(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && link.trim() && void load()}
          />
        </label>
        <Button variant="primary" busy={loading} disabled={!link.trim()} onClick={() => void load()}>
          Load their items
        </Button>
      </div>
      <div className="mt-1.5 text-[12px] text-faint">
        They find their link in Steam under Inventory, Trade Offers, "Who can send me Trade Offers?". The link is not written to the log.
      </div>
      {error && (
        <div className="mt-3">
          <Callout tone="bad">{error}</Callout>
        </div>
      )}

      {partner && (
        <>
          <div className="mt-5 flex flex-wrap gap-4">
            <Picker
              title="Your items"
              items={mine}
              picked={give}
              onToggle={toggleIn(setGive)}
              empty={inventory ? "None of your CS2 items can be traded right now." : "Loading your CS2 inventory…"}
            />
            <Picker title={`${playerName(partner.partner)}'s items`} items={theirs} picked={receive} onToggle={toggleIn(setReceive)} empty="None of their CS2 items can be traded right now." />
          </div>
          <div className="mt-4 flex flex-wrap items-end gap-3">
            <label className="min-w-[280px] flex-1 text-[12px] text-muted">
              A note to go with the offer (optional)
              <TextInput className="mt-1 w-full" maxLength={128} value={message} onChange={(e) => setMessage(e.target.value)} />
            </label>
            <Button
              variant="primary"
              icon={<Send size={14} />}
              disabled={giving.length + receiving.length === 0}
              onClick={() => {
                setChecked(false);
                setResult(null);
                setReview(true);
              }}
            >
              Review and send…
            </Button>
          </div>
        </>
      )}

      {review && partner && (
        <Modal title="Send this trade offer?" onClose={sending ? () => undefined : () => setReview(false)} width={560}>
          <div className="text-[13px] text-muted">To {playerName(partner.partner)}</div>
          <div className="mt-3 flex flex-wrap gap-4">
            <Side title="You give" items={asTradeItems(result?.ok ? [] : giving)} empty={result?.ok ? "Sent" : "Nothing"} />
            <Side title="You receive" items={asTradeItems(result?.ok ? [] : receiving)} empty={result?.ok ? "Sent" : "Nothing"} />
          </div>
          <div className="mt-3">
            {result ? <Callout tone={result.ok ? "ok" : "bad"}>{outcomeText(result, "Offer sent")}</Callout> : <Cautions give={giving.length} receive={receiving.length} />}
          </div>
          {!result?.ok && (
            <label className="mt-4 flex cursor-pointer items-start gap-2.5 text-[13px]">
              <input type="checkbox" className="mt-0.5 size-4 accent-[var(--ink)]" checked={checked} onChange={(e) => setChecked(e.target.checked)} />
              <span>I have looked at both sides and want to offer this trade.</span>
            </label>
          )}
          <div className="mt-4 flex justify-end gap-2">
            <Button onClick={() => setReview(false)} disabled={sending}>
              {result?.ok ? "Close" : "Cancel"}
            </Button>
            {!result?.ok && (
              <Button variant="primary" busy={sending} disabled={!checked} onClick={() => void send()}>
                Send the offer
              </Button>
            )}
          </div>
        </Modal>
      )}
    </div>
  );
}

export function TradesTab({ account }: { account: Account }) {
  const steamId = account.steamId;
  const { offers, offersLoading } = useCs2(steamId);
  const [sub, setSub] = useKept<Sub>("cs2.trade.sub", "received");
  const load = () => useCs2Store.getState().loadOffers(steamId);
  useEffect(() => {
    if (!offers && !offersLoading) void load();
    // Once per account, when the tab opens.
  }, [steamId]);

  const list = sub === "received" ? offers?.received : sub === "sent" ? offers?.sent : undefined;
  return (
    <div>
      <div className="mb-4 flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <Tabs
            value={sub}
            onChange={setSub}
            tabs={[
              { value: "received", label: "Received", count: offers?.received.length },
              { value: "sent", label: "Sent", count: offers?.sent.length },
              { value: "new", label: "New offer" },
            ]}
          />
        </div>
        {sub !== "new" && (
          <Button size="sm" icon={<RefreshCw size={13} className={cx(offersLoading && "animate-spin")} />} disabled={offersLoading} onClick={() => void load()}>
            Refresh
          </Button>
        )}
      </div>
      {sub === "new" ? (
        <NewOffer
          steamId={steamId}
          onSent={() => {
            void load();
          }}
        />
      ) : !offers ? (
        offersLoading ? (
          <Empty title="Loading trade offers…" body="The open offers other players sent you, and the ones you sent." />
        ) : (
          <Empty
            title="Load your trade offers"
            body="Shows the open offers other players sent you, and the ones you sent."
            action={
              <Button variant="primary" onClick={() => void load()}>
                Load trade offers
              </Button>
            }
          />
        )
      ) : list && list.length === 0 ? (
        <Empty
          title={sub === "received" ? "No open offers for you" : "No open offers from you"}
          body={sub === "received" ? "An offer another player sends you shows up here to accept or decline." : "An offer you send from New offer waits here until it is answered."}
        />
      ) : (
        <div className="flex flex-col gap-3">{list?.map((o) => <OfferCard key={o.id} steamId={steamId} offer={o} onChanged={() => void load()} />)}</div>
      )}
    </div>
  );
}
