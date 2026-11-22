import type { Hiding } from "../lib/hidden";
import { Button, Callout } from "./ui";

const games = (n: number) => `${n.toLocaleString()} ${n === 1 ? "game" : "games"}`;

/**
 * Says why games are missing from a list while "Hide NSFW games" is on and
 * Steam hasn't said yet whether they are ones to hide. Shows nothing once
 * every game is answered for, or with the setting off.
 */
export function HiddenNotice({ hiding, className }: { hiding: Hiding; className?: string }) {
  if (!hiding.on || (hiding.checking === 0 && hiding.unanswered === 0)) return null;
  if (hiding.unanswered === 0) {
    return (
      <p className={className ?? "mb-4 text-[13px] text-muted"} role="status">
        Asking Steam about {games(hiding.checking)} before showing {hiding.checking === 1 ? "it" : "them"}…
      </p>
    );
  }
  return (
    <div className={className ?? "mb-4"}>
      <Callout>
        <div className="font-medium">
          {games(hiding.unanswered)} {hiding.unanswered === 1 ? "isn't" : "aren't"} shown yet
        </div>
        <div className="mt-1 text-muted">
          “Hide NSFW games” is on, and Steam couldn't be asked whether {hiding.unanswered === 1 ? "this is one" : "these are ones"} to
          hide. {hiding.unanswered === 1 ? "It appears" : "They appear"} as soon as Steam answers.
        </div>
        <div className="mt-3">
          <Button size="sm" onClick={hiding.retry}>
            Ask again
          </Button>
        </div>
      </Callout>
    </div>
  );
}
