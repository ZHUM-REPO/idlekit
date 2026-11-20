import { Loader, X } from "lucide-react";
import { useEffect, useId, useRef, type ButtonHTMLAttributes, type ReactNode } from "react";

export function cx(...parts: (string | false | null | undefined)[]) {
  return parts.filter(Boolean).join(" ");
}

type Variant = "primary" | "quiet" | "ghost" | "danger";

export function Button({
  variant = "quiet",
  size = "md",
  busy,
  icon,
  className,
  children,
  disabled,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: Variant;
  size?: "sm" | "md";
  busy?: boolean;
  icon?: ReactNode;
}) {
  const styles: Record<Variant, string> = {
    primary: "bg-ink text-bg hover:bg-ink/85 border-transparent font-semibold",
    quiet: "bg-raised border-line hover:border-line-strong text-ink",
    ghost: "bg-transparent border-transparent hover:bg-raised text-muted hover:text-ink",
    danger: "bg-transparent border-line text-bad hover:bg-bad-soft hover:border-bad",
  };
  return (
    <button
      {...rest}
      disabled={disabled || busy}
      className={cx(
        "inline-flex items-center justify-center gap-1.5 rounded-md border whitespace-nowrap transition-colors",
        "disabled:opacity-45 disabled:pointer-events-none select-none",
        size === "sm" ? "h-7 px-2.5 text-[12px]" : "h-8.5 px-3.5 text-[13px]",
        styles[variant],
        className,
      )}
    >
      {busy ? <Loader size={14} className="animate-spin" /> : icon}
      {children}
    </button>
  );
}

export function IconButton({
  label,
  className,
  children,
  active,
  compact,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string; active?: boolean; compact?: boolean }) {
  return (
    <button
      {...rest}
      aria-label={label}
      title={label}
      className={cx(
        "inline-flex items-center justify-center rounded-md text-muted hover:bg-raised hover:text-ink transition-colors",
        compact ? "size-6.5" : "size-7.5",
        "disabled:opacity-40 disabled:pointer-events-none",
        active && "text-ink",
        className,
      )}
    >
      {children}
    </button>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cx(
        "relative inline-flex h-5 w-9 flex-none items-center rounded-full border transition-colors",
        checked ? "bg-ink border-transparent" : "bg-well border-line-strong",
        disabled && "opacity-40",
      )}
    >
      <span
        className={cx(
          "inline-block size-3.5 rounded-full transition-transform",
          checked ? "translate-x-[18px] bg-bg" : "translate-x-[2px] bg-muted",
        )}
      />
    </button>
  );
}

export function Field({
  label,
  hint,
  children,
  inline,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
  inline?: boolean;
}) {
  const id = useId();
  return (
    <div className={cx(inline ? "flex items-center justify-between gap-6 py-2.5" : "flex flex-col gap-1.5")}>
      <div className="min-w-0">
        <label htmlFor={id} className="block text-[13px] text-ink">
          {label}
        </label>
        {hint && <div className="mt-0.5 text-[12px] text-faint max-w-[60ch]">{hint}</div>}
      </div>
      <div id={id} className={cx(inline && "flex-none")}>
        {children}
      </div>
    </div>
  );
}

export const inputClass =
  "h-8.5 rounded-md border border-line bg-well px-2.5 text-[13px] text-ink placeholder:text-faint outline-none focus:border-line-strong focus:bg-raised";

export const smallInputClass =
  "h-7 rounded-md border border-line bg-well px-2 text-[12px] text-ink outline-none focus:border-line-strong";

export function TextInput(props: React.InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} className={cx(inputClass, props.className)} />;
}

export function NumberInput({
  value,
  onChange,
  min,
  max,
  step,
  className,
  suffix,
}: {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  step?: number;
  className?: string;
  suffix?: string;
}) {
  return (
    <span className="inline-flex items-center gap-2">
      <input
        type="number"
        value={Number.isFinite(value) ? value : 0}
        min={min}
        max={max}
        step={step}
        onChange={(e) => {
          const v = Number(e.target.value);
          if (!Number.isNaN(v)) onChange(v);
        }}
        className={cx(inputClass, "num w-20 text-right", className)}
      />
      {suffix && <span className="text-[12px] text-faint">{suffix}</span>}
    </span>
  );
}

export function Select<T extends string>({
  value,
  onChange,
  options,
  className,
  label,
  small,
  disabled,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
  className?: string;
  label?: string;
  small?: boolean;
  disabled?: boolean;
}) {
  return (
    <select
      aria-label={label}
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value as T)}
      className={cx(small ? smallInputClass : inputClass, "pr-7 cursor-pointer disabled:cursor-default disabled:opacity-45", className)}
    >
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

export type Tone = "run" | "ok" | "bad" | "neutral";

export function Pill({ tone = "neutral", children, lamp }: { tone?: Tone; children: ReactNode; lamp?: boolean }) {
  const styles: Record<Tone, string> = {
    run: "bg-run-soft text-run",
    ok: "bg-ok-soft text-ok",
    bad: "bg-bad-soft text-bad",
    neutral: "bg-raised text-muted",
  };
  return (
    <span className={cx("inline-flex h-5.5 items-center gap-1.5 rounded-full px-2 text-[12px] whitespace-nowrap", styles[tone])}>
      {lamp && <span className={cx("lamp", tone === "run" && "on", tone === "ok" && "ok", tone === "bad" && "bad")} />}
      {children}
    </span>
  );
}

export function Tabs<T extends string>({
  value,
  onChange,
  tabs,
}: {
  value: T;
  onChange: (v: T) => void;
  tabs: { value: T; label: string; count?: number }[];
}) {
  return (
    <div role="tablist" className="flex gap-1 border-b border-line">
      {tabs.map((t) => (
        <button
          key={t.value}
          role="tab"
          aria-selected={value === t.value}
          onClick={() => onChange(t.value)}
          className={cx(
            "relative -mb-px h-9 px-3 text-[13px] transition-colors border-b-2",
            value === t.value ? "border-ink text-ink" : "border-transparent text-muted hover:text-ink",
          )}
        >
          {t.label}
          {t.count !== undefined && <span className="num ml-1.5 text-faint">{t.count}</span>}
        </button>
      ))}
    </div>
  );
}

export function Modal({
  title,
  onClose,
  children,
  width = 460,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  width?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    ref.current?.querySelector<HTMLElement>("input,button,select")?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-6"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        style={{ width }}
        className="max-h-full overflow-auto rounded-xl border border-line-strong bg-panel shadow-2xl"
      >
        <div className="flex items-center justify-between border-b border-line px-5 py-3.5">
          <h2 className="text-[17px] font-semibold tracking-[-0.012em]">{title}</h2>
          <IconButton label="Close" onClick={onClose}>
            <X size={16} />
          </IconButton>
        </div>
        <div className="px-5 py-4">{children}</div>
      </div>
    </div>
  );
}

export function Empty({ title, body, action }: { title: string; body?: ReactNode; action?: ReactNode }) {
  return (
    <div className="flex flex-col items-start gap-2 rounded-lg border border-dashed border-line-strong px-6 py-8">
      <div className="text-[17px] font-semibold tracking-[-0.012em]">{title}</div>
      {body && <div className="max-w-[62ch] text-muted">{body}</div>}
      {action && <div className="mt-2">{action}</div>}
    </div>
  );
}

export function Callout({ tone = "neutral", children }: { tone?: Tone; children: ReactNode }) {
  const styles: Record<Tone, string> = {
    run: "border-run/40 bg-run-soft",
    ok: "border-ok/40 bg-ok-soft",
    bad: "border-bad/40 bg-bad-soft",
    neutral: "border-line bg-raised",
  };
  return <div className={cx("rounded-lg border px-4 py-3 text-[13px]", styles[tone])}>{children}</div>;
}

export function PageHeader({ title, sub, actions }: { title: string; sub?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-4 pb-5">
      <div className="min-w-0">
        <h1 className="heading text-[28px] font-semibold leading-none">{title}</h1>
        {sub && <div className="mt-2 max-w-[72ch] text-balance text-muted">{sub}</div>}
      </div>
      {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}

export function Section({
  title,
  aside,
  children,
  className,
}: {
  title?: string;
  aside?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cx("rounded-xl border border-line bg-panel", className)}>
      {title && (
        <div className="flex items-center justify-between gap-3 border-b border-line px-4 py-3">
          <h2 className="text-[15px] font-semibold tracking-[-0.008em]">{title}</h2>
          {aside}
        </div>
      )}
      {children}
    </section>
  );
}
