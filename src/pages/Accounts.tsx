import { Cloud, Cookie, KeyRound, Monitor, Plus, QrCode, Trash } from "lucide-react";
import QRCode from "qrcode";
import { useEffect, useState } from "react";
import { Avatar } from "../components/Shell";
import { Button, Callout, Field, Modal, PageHeader, Pill, Select, TextInput, cx, type Tone } from "../components/ui";
import { api, errorText } from "../lib/api";
import { useApp } from "../lib/store";
import type { Account, CmStatus, Mode } from "../lib/types";

const CM_LABEL: Record<CmStatus["status"], { text: string; tone: Tone }> = {
  online: { text: "Connected", tone: "ok" },
  blocked: { text: "Paused, playing elsewhere", tone: "run" },
  connecting: { text: "Connecting…", tone: "neutral" },
  reconnecting: { text: "Reconnecting…", tone: "neutral" },
  error: { text: "Connection problem", tone: "bad" },
  offline: { text: "Not connected", tone: "neutral" },
};

export function AccountsPage() {
  const accounts = useApp((s) => s.accounts);
  const local = useApp((s) => s.localSteam);
  const [adding, setAdding] = useState(false);
  const [webFor, setWebFor] = useState<Account | null>(null);

  const localLine = !local?.installPath
    ? "Steam isn't installed on this PC (or IdleKit couldn't find it). Direct connections still work."
    : !local.running
      ? "The Steam client isn't running. Start it to idle through it or to change achievements."
      : local.activeSteamId
        ? `The Steam client is signed in as ${local.users.find((u) => u.steamId === local.activeSteamId)?.accountName ?? local.activeSteamId}.`
        : "The Steam client is running but nobody is signed in.";

  return (
    <div className="pb-10">
      <PageHeader
        title="Accounts"
        sub={localLine}
        actions={
          <Button variant="primary" icon={<Plus size={15} />} onClick={() => setAdding(true)}>
            Add account
          </Button>
        }
      />

      {accounts.length === 0 ? (
        <div className="grid gap-4 md:grid-cols-3">
          <MethodCard
            icon={<Monitor size={18} />}
            title="Steam client on this PC"
            body="Uses the account already signed in to Steam here. Needed for changing achievements. One account at a time."
          />
          <MethodCard
            icon={<Cloud size={18} />}
            title="Direct connection"
            body="IdleKit signs in to Steam itself, so several accounts can idle at once with Steam closed."
          />
          <MethodCard
            icon={<Cookie size={18} />}
            title="Web session"
            body="Card farming and the market read steamcommunity.com, which needs a sign-in or a pasted cookie."
          />
          <div className="md:col-span-3">
            <Button variant="primary" icon={<Plus size={15} />} onClick={() => setAdding(true)}>
              Add your first account
            </Button>
          </div>
        </div>
      ) : (
        <div className="overflow-hidden rounded-xl border border-line bg-panel">
          {accounts.map((a, i) => (
            <AccountRow key={a.steamId} account={a} first={i === 0} onWeb={() => setWebFor(a)} />
          ))}
        </div>
      )}

      {adding && <AddAccountModal onClose={() => setAdding(false)} />}
      {webFor && <WebSessionModal account={webFor} onClose={() => setWebFor(null)} />}
    </div>
  );
}

function MethodCard({ icon, title, body }: { icon: React.ReactNode; title: string; body: string }) {
  return (
    <div className="rounded-xl border border-line bg-panel px-4 py-4">
      <div className="mb-2 flex items-center gap-2 text-ink">
        <span className="text-faint">{icon}</span>
        <span className="font-medium">{title}</span>
      </div>
      <p className="text-[13px] text-muted">{body}</p>
    </div>
  );
}

function AccountRow({ account: a, first, onWeb }: { account: Account; first: boolean; onWeb: () => void }) {
  const cm = useApp((s) => s.cm[a.steamId]);
  const activeId = useApp((s) => s.activeId);
  const setActive = useApp((s) => s.setActive);
  const run = useApp((s) => s.run);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const cmState = CM_LABEL[cm?.status ?? "offline"];

  const setMode = async (mode: Mode) => {
    if (mode === a.mode) return;
    setBusy("mode");
    await run(mode === "direct" ? "Switched to a direct connection" : "Switched to the Steam client", () =>
      api.setAccountMode(a.steamId, mode),
    );
    setBusy(null);
  };

  return (
    <div className={cx("px-5 py-4", !first && "border-t border-line")}>
      <div className="flex flex-wrap items-center gap-4">
        <Avatar account={a} size={40} />
        <div className="min-w-[180px] flex-1">
          <div className="flex items-center gap-2">
            <span className="text-[15px] font-medium">{a.personaName || a.accountName}</span>
            {a.steamId === activeId && <Pill>Selected</Pill>}
          </div>
          <div className="text-[12px] text-faint">
            {a.accountName}, SteamID {a.steamId}
          </div>
        </div>

        <div role="radiogroup" aria-label="How this account idles" className="flex rounded-lg border border-line bg-well p-0.5">
          {(["local", "direct"] as Mode[]).map((m) => (
            <button
              key={m}
              role="radio"
              aria-checked={a.mode === m}
              disabled={busy === "mode"}
              onClick={() => void setMode(m)}
              className={cx(
                "flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[12px]",
                a.mode === m ? "bg-raised text-ink" : "text-muted hover:text-ink",
              )}
            >
              {m === "local" ? <Monitor size={13} /> : <Cloud size={13} />}
              {m === "local" ? "Steam client" : "Direct"}
            </button>
          ))}
        </div>

        {a.steamId !== activeId && (
          <Button size="sm" onClick={() => void setActive(a.steamId)}>
            Select
          </Button>
        )}
        <Button size="sm" variant="ghost" aria-label={`Remove ${a.accountName}`} onClick={() => setConfirmRemove(true)}>
          <Trash size={14} />
        </Button>
      </div>

      <div className="mt-3 grid gap-3 pl-14 xl:grid-cols-2">
        <div className="flex flex-wrap items-center gap-2.5 text-[13px]">
          <span className="w-[84px] flex-none text-faint">Idling</span>
          {a.mode === "local" ? (
            <Pill tone={a.localActive ? "ok" : "neutral"} lamp>
              {a.localActive ? "Steam client signed in" : "Steam client not signed in to this account"}
            </Pill>
          ) : (
            <>
              <Pill tone={cmState.tone} lamp>
                {cmState.text}
              </Pill>
              {cm?.status === "online" || cm?.status === "blocked" ? (
                <Button
                  size="sm"
                  variant="ghost"
                  busy={busy === "conn"}
                  onClick={async () => {
                    setBusy("conn");
                    await run(null, () => api.disconnect(a.steamId));
                    setBusy(null);
                  }}
                >
                  Disconnect
                </Button>
              ) : (
                <Button
                  size="sm"
                  busy={busy === "conn" || cm?.status === "connecting"}
                  disabled={!a.hasToken}
                  onClick={async () => {
                    setBusy("conn");
                    await run(null, () => api.connect(a.steamId));
                    setBusy(null);
                  }}
                >
                  Connect
                </Button>
              )}
              <Select
                label="Online status"
                value={String(a.personaState)}
                onChange={(v) => void run(null, () => api.setPersonaState(a.steamId, Number(v)))}
                small
                options={[
                  { value: "1", label: "Online" },
                  { value: "3", label: "Away" },
                  { value: "7", label: "Invisible" },
                  { value: "0", label: "Offline" },
                ]}
              />
            </>
          )}
          {cm?.message && a.mode === "direct" && <span className="w-full text-[12px] text-faint">{cm.message}</span>}
          {a.mode === "direct" && !a.hasToken && (
            <span className="w-full text-[12px] text-faint">Sign in with a QR code or password to connect.</span>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-2.5 text-[13px]">
          <span className="w-[84px] flex-none text-faint">Web session</span>
          {a.hasToken ? (
            <Pill tone="ok" lamp>
              Signed in
            </Pill>
          ) : a.hasCookie ? (
            <Pill tone="ok" lamp>
              Using a pasted cookie
            </Pill>
          ) : (
            <Pill tone="neutral" lamp>
              None
            </Pill>
          )}
          <Button size="sm" onClick={onWeb}>
            {a.hasToken || a.hasCookie ? "Change" : "Set up"}
          </Button>
          {(a.hasToken || a.hasCookie) && (
            <Button
              size="sm"
              variant="ghost"
              busy={busy === "check"}
              onClick={async () => {
                setBusy("check");
                await run("Web session works", () => api.webCheck(a.steamId));
                setBusy(null);
              }}
            >
              Test
            </Button>
          )}
        </div>
      </div>

      {confirmRemove && (
        <Modal title={`Remove ${a.accountName}?`} onClose={() => setConfirmRemove(false)}>
          <div className="flex flex-col gap-4">
            <p className="text-muted">
              Stops everything running for this account and deletes its saved sign-in from this PC. Your Steam account
              itself isn't affected.
            </p>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setConfirmRemove(false)}>
                Cancel
              </Button>
              <Button
                variant="danger"
                busy={busy === "remove"}
                onClick={async () => {
                  setBusy("remove");
                  await run("Account removed", () => api.removeAccount(a.steamId));
                  setBusy(null);
                  setConfirmRemove(false);
                }}
              >
                Remove account
              </Button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}

// ------------------------------------------------------------- sign in ---

type Method = "local" | "qr" | "password";

function AddAccountModal({ onClose }: { onClose: () => void }) {
  const local = useApp((s) => s.localSteam);
  const accounts = useApp((s) => s.accounts);
  const run = useApp((s) => s.run);
  const [method, setMethod] = useState<Method>(local?.users.length ? "local" : "qr");
  const [mode, setMode] = useState<Mode>("direct");

  return (
    <Modal title="Add a Steam account" onClose={onClose} width={560}>
      <div role="tablist" className="mb-4 grid grid-cols-3 gap-1 rounded-lg border border-line bg-well p-1">
        {(
          [
            ["local", "This PC's Steam", <Monitor key="m" size={14} />],
            ["qr", "QR code", <QrCode key="q" size={14} />],
            ["password", "Password", <KeyRound key="k" size={14} />],
          ] as [Method, string, React.ReactNode][]
        ).map(([m, label, icon]) => (
          <button
            key={m}
            role="tab"
            aria-selected={method === m}
            onClick={() => setMethod(m)}
            className={cx(
              "flex h-8 items-center justify-center gap-1.5 rounded-md text-[13px]",
              method === m ? "bg-raised text-ink" : "text-muted hover:text-ink",
            )}
          >
            {icon}
            {label}
          </button>
        ))}
      </div>

      {method === "local" ? (
        <div className="flex flex-col gap-3">
          <p className="text-muted">
            Accounts that have signed in to the Steam client on this PC. IdleKit idles through the client and never sees
            your password.
          </p>
          {(local?.users ?? []).length === 0 ? (
            <Callout>No saved Steam logins were found on this PC.</Callout>
          ) : (
            <div className="overflow-hidden rounded-lg border border-line">
              {local!.users.map((u, i) => {
                const added = accounts.some((a) => a.steamId === u.steamId);
                return (
                  <div key={u.steamId} className={cx("flex items-center gap-3 px-3 py-2.5", i > 0 && "border-t border-line")}>
                    <div className="min-w-0 flex-1">
                      <div>{u.personaName || u.accountName}</div>
                      <div className="text-[12px] text-faint">
                        {u.accountName}
                        {u.active ? ", signed in now" : ""}
                      </div>
                    </div>
                    <Button
                      size="sm"
                      variant={added ? "ghost" : "primary"}
                      disabled={added}
                      onClick={async () => {
                        await run("Account added", () => api.addLocalAccount(u.steamId));
                        onClose();
                      }}
                    >
                      {added ? "Added" : "Add"}
                    </Button>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      ) : (
        <div className="flex flex-col gap-4">
          <Field label="Idle this account through">
            <Select
              value={mode}
              onChange={setMode}
              options={[
                { value: "direct", label: "A direct connection (no Steam client needed)" },
                { value: "local", label: "The Steam client on this PC" },
              ]}
            />
          </Field>
          <SignIn method={method} mode={mode} onDone={onClose} />
        </div>
      )}
    </Modal>
  );
}

function WebSessionModal({ account, onClose }: { account: Account; onClose: () => void }) {
  const [method, setMethod] = useState<"qr" | "password" | "cookie">("qr");
  const [cookie, setCookie] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <Modal title={`Web session for ${account.accountName}`} onClose={onClose} width={560}>
      <p className="mb-4 text-muted">
        Card farming and the market read steamcommunity.com as you. Signing in also lets this account use a direct
        connection later.
      </p>
      <div role="tablist" className="mb-4 grid grid-cols-3 gap-1 rounded-lg border border-line bg-well p-1">
        {(
          [
            ["qr", "QR code"],
            ["password", "Password"],
            ["cookie", "Paste cookie"],
          ] as const
        ).map(([m, label]) => (
          <button
            key={m}
            role="tab"
            aria-selected={method === m}
            onClick={() => setMethod(m)}
            className={cx("h-8 rounded-md text-[13px]", method === m ? "bg-raised text-ink" : "text-muted hover:text-ink")}
          >
            {label}
          </button>
        ))}
      </div>
      {method === "cookie" ? (
        <div className="flex flex-col gap-3">
          <Field
            label="steamLoginSecure cookie"
            hint="In a browser signed in to steamcommunity.com, open developer tools, find Cookies, and copy the value of steamLoginSecure. It expires after about a day."
          >
            <textarea
              value={cookie}
              onChange={(e) => setCookie(e.target.value)}
              rows={3}
              placeholder="7656119…%7C%7CeyAidHlwIjogIkpXVCIsICJhbGciOiAiRWREU0EiIH0…"
              className="w-full rounded-md border border-line bg-well px-2.5 py-2 font-mono text-[12px] outline-none focus:border-line-strong"
            />
          </Field>
          {error && <Callout tone="bad">{error}</Callout>}
          <div className="flex justify-end">
            <Button
              variant="primary"
              busy={busy}
              disabled={!cookie.trim()}
              onClick={async () => {
                setBusy(true);
                setError(null);
                try {
                  await api.setWebCookie(account.steamId, cookie);
                  useApp.getState().toast("Web session saved", "ok");
                  onClose();
                } catch (e) {
                  setError(errorText(e));
                }
                setBusy(false);
              }}
            >
              Check and save
            </Button>
          </div>
        </div>
      ) : (
        <SignIn method={method} mode={account.mode} onDone={onClose} expectSteamId={account.steamId} />
      )}
    </Modal>
  );
}

function SignIn({
  method,
  mode,
  onDone,
  expectSteamId,
}: {
  method: "qr" | "password";
  mode: Mode;
  onDone: () => void;
  expectSteamId?: string;
}) {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const ev = useApp((s) => (sessionId ? s.auth[sessionId] : undefined));
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [svg, setSvg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const startQr = async () => {
    setError(null);
    setSvg(null);
    try {
      setSessionId(await api.authStartQr(mode));
    } catch (e) {
      setError(errorText(e));
    }
  };

  useEffect(() => {
    if (method === "qr") void startQr();
    return () => {
      setSessionId((id) => {
        if (id) void api.authCancel(id);
        return null;
      });
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [method]);

  useEffect(() => {
    if (ev?.challengeUrl) {
      QRCode.toString(ev.challengeUrl, { type: "svg", margin: 1, errorCorrectionLevel: "M", color: { dark: "#111111", light: "#ffffff" } })
        .then(setSvg)
        .catch(() => setSvg(null));
    }
  }, [ev?.challengeUrl]);

  useEffect(() => {
    if (ev?.status === "done") {
      const wrong = expectSteamId && ev.steamId && ev.steamId !== expectSteamId;
      useApp
        .getState()
        .toast(
          wrong ? `Signed in as ${ev.accountName}, which is a different account; it was added separately` : `Signed in as ${ev.accountName}`,
          wrong ? "info" : "ok",
        );
      onDone();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ev?.status]);

  const status = ev?.status;

  if (method === "qr") {
    return (
      <div className="flex gap-5">
        <div className="flex size-[196px] flex-none items-center justify-center rounded-lg bg-white p-2">
          {svg && status !== "expired" && status !== "error" ? (
            <div className="size-full [&>svg]:size-full" dangerouslySetInnerHTML={{ __html: svg }} />
          ) : (
            <span className="text-[12px] text-neutral-500">{status === "expired" ? "Code expired" : "Getting a code…"}</span>
          )}
        </div>
        <div className="flex flex-col gap-3 text-[13px]">
          <p>
            Open the Steam Mobile app, tap the shield (Steam Guard), then <span className="text-ink">Scan a QR code</span>, and
            point it at this code.
          </p>
          <p className="text-muted">
            {status === "scanned"
              ? "Scanned. Approve the sign-in in the app."
              : status === "working"
                ? "Finishing sign-in…"
                : "Waiting for a scan. The code refreshes on its own."}
          </p>
          {(status === "expired" || status === "error" || error) && (
            <>
              {(ev?.message || error) && <Callout tone="bad">{ev?.message || error}</Callout>}
              <div>
                <Button size="sm" onClick={() => void startQr()}>
                  Get a new code
                </Button>
              </div>
            </>
          )}
        </div>
      </div>
    );
  }

  const waitingForCode = status === "code" || status === "confirm";
  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={async (e) => {
        e.preventDefault();
        setError(null);
        if (waitingForCode && sessionId) {
          try {
            await api.authSubmitCode(sessionId, code);
          } catch (err) {
            setError(errorText(err));
          }
          return;
        }
        try {
          setSessionId(await api.authStartCredentials(username, password, mode));
        } catch (err) {
          setError(errorText(err));
        }
      }}
    >
      {!waitingForCode ? (
        <>
          <Field label="Steam username">
            <TextInput value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="username" className="w-full" />
          </Field>
          <Field label="Password" hint="Sent straight to Steam. IdleKit keeps only the sign-in token Steam returns, in your OS credential store.">
            <TextInput
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              className="w-full"
            />
          </Field>
        </>
      ) : status === "confirm" ? (
        <Callout tone="run">Approve this sign-in in the Steam Mobile app, or from the link Steam emailed you.</Callout>
      ) : (
        <Field
          label={ev?.codeType === "device" ? "Steam Guard code from the mobile app" : `Code emailed to you${ev?.hint ? ` at ${ev.hint}` : ""}`}
          hint={ev?.canConfirm ? "Or approve the sign-in in the Steam Mobile app instead." : undefined}
        >
          <TextInput
            value={code}
            onChange={(e) => setCode(e.target.value.toUpperCase())}
            autoFocus
            maxLength={8}
            className="num w-40 text-center text-[18px] tracking-[0.2em]"
          />
        </Field>
      )}
      {(error || (status === "error" && ev?.message)) && <Callout tone="bad">{error || ev?.message}</Callout>}
      {status !== "confirm" && (
        <div className="flex justify-end">
          <Button
            type="submit"
            variant="primary"
            busy={status === "working"}
            disabled={waitingForCode ? code.trim().length < 5 : !username || !password}
          >
            {waitingForCode ? "Submit code" : "Sign in"}
          </Button>
        </div>
      )}
    </form>
  );
}
