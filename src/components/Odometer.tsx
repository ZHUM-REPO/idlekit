import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { cx } from "./ui";

const DIGITS = Array.from({ length: 20 }, (_, i) => i % 10);

/** One rolling digit. Rolls forward through 9→0 like a mechanical counter. */
function Drum({ digit }: { digit: number }) {
  const [pos, setPos] = useState(digit);
  const [animate, setAnimate] = useState(true);
  const prev = useRef(digit);

  useLayoutEffect(() => {
    if (digit === prev.current) return;
    setAnimate(true);
    setPos(digit < prev.current ? digit + 10 : digit);
    prev.current = digit;
  }, [digit]);

  useEffect(() => {
    if (animate) return;
    // Re-enable transitions on the frame after a silent snap-back.
    const id = requestAnimationFrame(() => setAnimate(true));
    return () => cancelAnimationFrame(id);
  }, [animate]);

  return (
    <span className="odo-drum" aria-hidden="true">
      <span
        className={cx("odo-strip", !animate && "no-anim")}
        style={{ transform: `translateY(${-pos * 1.2}em)` }}
        onTransitionEnd={() => {
          if (pos >= 10) {
            setAnimate(false);
            setPos(pos - 10);
          }
        }}
      >
        {DIGITS.map((d, i) => (
          <span key={i}>{d}</span>
        ))}
      </span>
    </span>
  );
}

/**
 * Renders text like "12:04:09" or "317" with each digit on its own drum.
 * Non-digit characters render as separators.
 */
export function Odometer({
  text,
  className,
  live,
  label,
}: {
  text: string;
  className?: string;
  live?: boolean;
  label?: string;
}) {
  const chars = [...text];
  return (
    <span className={cx("odo", live && "odo-live", className)} role="img" aria-label={label ?? text}>
      {chars.map((c, i) =>
        /\d/.test(c) ? (
          // Key from the right so digits keep their drum when the length changes.
          <Drum key={`d${chars.length - i}`} digit={Number(c)} />
        ) : (
          <span key={`s${chars.length - i}`} className="odo-sep" aria-hidden="true">
            {c}
          </span>
        ),
      )}
    </span>
  );
}
