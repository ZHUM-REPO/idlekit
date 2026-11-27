//! Pricing a whole inventory without upsetting Steam's market. Requests are
//! paced by when they start, with a few waiting for their answer at once, so
//! one slow answer doesn't hold up the rest; "too many requests" pauses all
//! of them and slows the pace; and what was learned is kept on disk for an
//! hour, so opening the market again doesn't ask Steam again.
use super::market::{Book, Wallet};
use futures_util::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::path::Path;
use std::time::Duration;
use tokio::time::Instant;

/// The price list's file, in the data folder.
pub const FILE: &str = "market_prices.json";
/// A price is checked again by itself once it is this old.
pub const FRESH_FOR_MS: i64 = 60 * 60 * 1000;
/// Prices this old are forgotten.
const FORGET_AFTER_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// An account's wallet is read again from the market page once it is this old.
pub const WALLET_FRESH_FOR_MS: i64 = 24 * 60 * 60 * 1000;
/// A time this far ahead of the clock is a clock that was wrong, not the future.
const CLOCK_SLACK_MS: i64 = 60 * 1000;

// ------------------------------------------------------------------ kept --

/// An item's order book as last seen.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
    pub lowest_sell: Option<u32>,
    pub highest_buy: Option<u32>,
    pub sell_listings: u32,
    /// When Steam was asked, in milliseconds since 1970.
    pub checked_at: i64,
}

impl Seen {
    pub fn of(book: &Book, checked_at: i64) -> Seen {
        Seen {
            lowest_sell: book.lowest_sell,
            highest_buy: book.highest_buy,
            sell_listings: book.sell_listings,
            checked_at,
        }
    }

    /// Recent enough not to ask Steam again.
    pub fn fresh(&self, now: i64) -> bool {
        is_recent(self.checked_at, now, FRESH_FOR_MS)
    }
}

fn is_recent(at: i64, now: i64, within: i64) -> bool {
    now - at < within && at <= now + CLOCK_SLACK_MS
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeenWallet {
    pub wallet: Wallet,
    pub checked_at: i64,
}

impl SeenWallet {
    pub fn fresh(&self, now: i64) -> bool {
        is_recent(self.checked_at, now, WALLET_FRESH_FOR_MS)
    }
}

/// What IdleKit knows of the market's prices: by currency, then by item; and
/// each account's wallet, by Steam ID.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PriceList {
    books: BTreeMap<u32, HashMap<String, Seen>>,
    wallets: HashMap<String, SeenWallet>,
}

impl PriceList {
    /// The list kept on disk, without what is too old to be of use. Nothing
    /// (or something that can't be read) is an empty list.
    pub fn load(file: &Path, now: i64) -> PriceList {
        let mut list: PriceList = std::fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        list.forget_old(now);
        list
    }

    /// Written whole, then put in place, so a crash halfway leaves the old file.
    pub fn save(bytes: &[u8], file: &Path) -> anyhow::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, file)?;
        Ok(())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    fn forget_old(&mut self, now: i64) {
        for books in self.books.values_mut() {
            books.retain(|_, seen| is_recent(seen.checked_at, now, FORGET_AFTER_MS));
        }
        self.books.retain(|_, books| !books.is_empty());
    }

    pub fn get(&self, currency: u32, name: &str) -> Option<Seen> {
        self.books.get(&currency)?.get(name).copied()
    }

    pub fn put(&mut self, currency: u32, name: &str, seen: Seen) {
        self.books.entry(currency).or_default().insert(name.to_string(), seen);
    }

    pub fn wallet(&self, steam_id: &str) -> Option<SeenWallet> {
        self.wallets.get(steam_id).copied()
    }

    pub fn put_wallet(&mut self, steam_id: &str, wallet: Wallet, checked_at: i64) {
        self.wallets.insert(steam_id.to_string(), SeenWallet { wallet, checked_at });
    }

    pub fn forget_wallet(&mut self, steam_id: &str) {
        self.wallets.remove(steam_id);
    }
}

// ---------------------------------------------------------------- asking --

/// What asking Steam about one item came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Book(Book),
    /// Steam has no market for this item.
    NotOnMarket,
    /// "Too many requests": every request waits, and the item is asked again.
    Busy,
    /// Steam, or the connection, had trouble: the item is asked again later.
    Trouble(String),
    /// The item can't be priced, and why.
    Failed(String),
}

/// How fast to ask.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    /// From the start of one request to the start of the next.
    pub every: Duration,
    /// Requests waiting for their answer at the same time, at most.
    pub at_once: usize,
}

/// When the next request may start, and the pause Steam last asked for.
/// Runs take turns with it, so a new one doesn't rush in where the one
/// before it was told to wait.
#[derive(Debug)]
pub struct Pacer {
    next_start: Option<Instant>,
    paused_until: Option<Instant>,
    next_pause: Duration,
}

impl Default for Pacer {
    fn default() -> Self {
        Pacer { next_start: None, paused_until: None, next_pause: FIRST_PAUSE }
    }
}

/// The pause after a first "too many requests"; each one after it, with no
/// answer in between, is twice as long as the one before.
const FIRST_PAUSE: Duration = Duration::from_secs(20);
const LONGEST_PAUSE: Duration = Duration::from_secs(160);
/// Pauses in a row, with no answer in between, before a run gives up.
const PAUSES_BEFORE_GIVING_UP: u32 = 4;
/// Each pause makes the pace half again as slow, down to this.
const SLOWEST: Duration = Duration::from_secs(8);
/// Times an item is asked about while the answers are trouble.
const TRIES: u8 = 3;
/// Troubles in a row, for whichever items, that mean Steam is having a bad
/// time: they make a pause too.
const TROUBLES_FOR_A_PAUSE: u32 = 5;
/// How often a waiting run looks at whether it should stop.
const LOOK_EVERY: Duration = Duration::from_millis(250);
/// After a stop, how long the requests already sent get to finish.
const FINISH_WITHIN: Duration = Duration::from_secs(3);

/// How a run ended.
#[derive(Debug, PartialEq)]
pub enum RunEnd {
    /// Every item has its answer.
    Done,
    /// Stopped from outside: cancelled, or a newer run took over.
    Stopped,
    /// Steam kept saying "too many requests"; this many items weren't priced.
    GaveUp { left: usize },
}

/// What a run tells its caller as it goes.
#[derive(Debug, PartialEq)]
pub enum News<'a> {
    /// An item's last word: a book, not on the market, or failed.
    Answered(&'a str, Answer),
    /// Steam asked for a pause: how long it is, and the pace after it.
    Pausing { pause: Duration, every: Duration },
}

/// Asks about every item in `names`, first to last, at `pace`, and tells
/// `news` each item's answer as it comes. `stop` is looked at often; once it
/// says so, the run ends, after the answers already on their way.
pub async fn run<A, Fut>(
    names: Vec<String>,
    pace: Pace,
    pacer: &mut Pacer,
    stop: impl Fn() -> bool,
    ask: A,
    mut news: impl FnMut(News<'_>),
) -> RunEnd
where
    A: Fn(String) -> Fut,
    Fut: Future<Output = Answer>,
{
    let at_once = pace.at_once.max(1);
    let mut every = pace.every;
    let mut queue: VecDeque<(String, u8)> = names.into_iter().map(|name| (name, 0)).collect();
    let mut asking = FuturesUnordered::new();
    // Pauses with no answer between them, and troubles in a row.
    let mut pauses = 0u32;
    let mut troubles = 0u32;
    // When the latest pause began: an answer to a request sent before it
    // was refused for the same reason, and makes no pause of its own. Items
    // refused go back to the front of the queue, in the order they were in.
    let mut pause_began: Option<Instant> = None;
    let mut refused = 0usize;
    loop {
        if stop() {
            let _ = tokio::time::timeout(FINISH_WITHIN, async {
                loop {
                    let got: Option<(String, u8, Instant, Answer)> = asking.next().await;
                    let Some((name, _, _, answer)) = got else { break };
                    if !matches!(answer, Answer::Busy | Answer::Trouble(_)) {
                        news(News::Answered(&name, answer));
                    }
                }
            })
            .await;
            return RunEnd::Stopped;
        }
        if queue.is_empty() && asking.is_empty() {
            return RunEnd::Done;
        }
        let now = Instant::now();
        let ready_at = pacer.next_start.into_iter().chain(pacer.paused_until).max();
        let room = !queue.is_empty() && asking.len() < at_once;
        if room && ready_at.is_none_or(|t| t <= now) {
            let (name, tries) = queue.pop_front().expect("the queue has an item");
            pacer.next_start = Some(now + every);
            let answer = ask(name.clone());
            asking.push(async move { (name, tries, now, answer.await) });
            continue;
        }
        let mut wake = now + LOOK_EVERY;
        if let (true, Some(t)) = (room, ready_at) {
            wake = wake.min(t);
        }
        let got = tokio::select! {
            got = asking.next(), if !asking.is_empty() => got,
            _ = tokio::time::sleep_until(wake) => None,
        };
        let Some((name, tries, sent, answer)) = got else { continue };
        let new_pause = pause_began.is_none_or(|began| sent >= began);
        match answer {
            Answer::Busy => {
                if !new_pause {
                    queue.insert(refused.min(queue.len()), (name, tries));
                    refused += 1;
                    continue;
                }
                queue.push_front((name, tries));
                refused = 1;
                pauses += 1;
                if pauses > PAUSES_BEFORE_GIVING_UP {
                    return RunEnd::GaveUp { left: queue.len() + asking.len() };
                }
                let pause = take_a_pause(pacer, &mut every);
                pause_began = Some(Instant::now());
                news(News::Pausing { pause, every });
            }
            Answer::Trouble(why) => {
                if tries + 1 < TRIES {
                    queue.push_back((name, tries + 1));
                } else {
                    news(News::Answered(&name, Answer::Failed(why)));
                }
                troubles += 1;
                if troubles >= TROUBLES_FOR_A_PAUSE && new_pause {
                    troubles = 0;
                    refused = 0;
                    pauses += 1;
                    if pauses > PAUSES_BEFORE_GIVING_UP {
                        return RunEnd::GaveUp { left: queue.len() + asking.len() };
                    }
                    let pause = take_a_pause(pacer, &mut every);
                    pause_began = Some(Instant::now());
                    news(News::Pausing { pause, every });
                }
            }
            answer => {
                // Steam answered: whatever went wrong before is over.
                pauses = 0;
                troubles = 0;
                pacer.next_pause = FIRST_PAUSE;
                news(News::Answered(&name, answer));
            }
        }
    }
}

/// Starts a pause, and slows the pace for after it. The length of the pause.
fn take_a_pause(pacer: &mut Pacer, every: &mut Duration) -> Duration {
    let pause = pacer.next_pause;
    pacer.paused_until = Some(Instant::now() + pause);
    pacer.next_pause = (pause * 2).min(LONGEST_PAUSE);
    *every = (*every * 3 / 2).min(SLOWEST).max(*every);
    pause
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn book(cents: u32) -> Book {
        Book { currency: 31, lowest_sell: Some(cents), highest_buy: Some(cents - 1), sell_listings: 5 }
    }

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("item {i}")).collect()
    }

    /// What a run did: when each request started (seconds after the run
    /// began), the most at once, and its news.
    #[derive(Default)]
    struct Record {
        starts: Vec<(String, f64)>,
        most_at_once: usize,
        answered: Vec<(String, Answer)>,
        pauses: Vec<(Duration, Duration)>,
    }

    /// Runs `names` with answers from `answer` (the item and how many times
    /// it was asked before), each taking `takes` to come.
    async fn try_run(
        names: Vec<String>,
        pace: Pace,
        takes: Duration,
        answer: impl Fn(&str, usize) -> Answer,
        stop_after: Option<usize>,
    ) -> (RunEnd, Record) {
        let began = Instant::now();
        let seen = Rc::new(RefCell::new(Record::default()));
        let waiting = Rc::new(RefCell::new(0usize));
        let asked: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
        let mut pacer = Pacer::default();
        let end = run(
            names,
            pace,
            &mut pacer,
            || stop_after.is_some_and(|n| seen.borrow().answered.len() >= n),
            |name: String| {
                let before = {
                    let mut asked = asked.borrow_mut();
                    let n = asked.entry(name.clone()).or_default();
                    *n += 1;
                    *n - 1
                };
                let reply = answer(&name, before);
                let (seen, waiting) = (seen.clone(), waiting.clone());
                {
                    let mut s = seen.borrow_mut();
                    s.starts.push((name, began.elapsed().as_secs_f64()));
                    *waiting.borrow_mut() += 1;
                    s.most_at_once = s.most_at_once.max(*waiting.borrow());
                }
                async move {
                    tokio::time::sleep(takes).await;
                    *waiting.borrow_mut() -= 1;
                    reply
                }
            },
            |news| match news {
                News::Answered(name, answer) => seen.borrow_mut().answered.push((name.to_string(), answer)),
                News::Pausing { pause, every } => seen.borrow_mut().pauses.push((pause, every)),
            },
        )
        .await;
        let seen = Rc::try_unwrap(seen).ok().expect("the run is over").into_inner();
        (end, seen)
    }

    const PACE: Pace = Pace { every: Duration::from_millis(1500), at_once: 3 };

    #[tokio::test(start_paused = true)]
    async fn requests_start_a_pace_apart_however_long_answers_take() {
        // Answers that take longer than the pace: the next request starts anyway.
        let (end, seen) = try_run(names(6), PACE, Duration::from_millis(4000), |_, _| Answer::Book(book(21)), None).await;
        assert_eq!(end, RunEnd::Done);
        let starts: Vec<f64> = seen.starts.iter().map(|s| s.1).collect();
        // 0, 1.5, 3: three waiting. The first answer comes at 4: the fourth
        // starts then, and so on, never closer than the pace.
        assert_eq!(starts, vec![0.0, 1.5, 3.0, 4.5, 6.0, 7.5]);
        assert_eq!(seen.most_at_once, 3);
        assert_eq!(seen.answered.len(), 6);
        assert_eq!(seen.answered[0], ("item 0".into(), Answer::Book(book(21))));
    }

    #[tokio::test(start_paused = true)]
    async fn no_more_than_the_limit_wait_at_once() {
        let (end, seen) = try_run(names(5), PACE, Duration::from_secs(10), |_, _| Answer::NotOnMarket, None).await;
        assert_eq!(end, RunEnd::Done);
        let starts: Vec<f64> = seen.starts.iter().map(|s| s.1).collect();
        // Three by 3 s; the fourth waits for the first answer, at 10 s.
        assert_eq!(starts, vec![0.0, 1.5, 3.0, 10.0, 11.5]);
        assert_eq!(seen.most_at_once, 3);
    }

    #[tokio::test(start_paused = true)]
    async fn too_many_requests_pauses_everything_and_asks_again() {
        // item 1's first answer is "too many requests".
        let (end, seen) = try_run(
            names(4),
            Pace { every: Duration::from_secs(1), at_once: 1 },
            Duration::from_millis(200),
            |name, before| if name == "item 1" && before == 0 { Answer::Busy } else { Answer::Book(book(30)) },
            None,
        )
        .await;
        assert_eq!(end, RunEnd::Done);
        assert_eq!(seen.pauses, vec![(Duration::from_secs(20), Duration::from_millis(1500))]);
        let starts: Vec<(String, f64)> = seen.starts.clone();
        // item 1 is refused at 1.2 s: nothing starts until 21.2 s, when
        // item 1 is asked again, first; then the slower pace.
        assert_eq!(starts[0], ("item 0".into(), 0.0));
        assert_eq!(starts[1], ("item 1".into(), 1.0));
        assert_eq!(starts[2].0, "item 1");
        assert!((starts[2].1 - 21.2).abs() < 1e-6, "{starts:?}");
        assert_eq!(starts[3].0, "item 2");
        assert!((starts[3].1 - 22.7).abs() < 1e-6, "{starts:?}");
        let answered: Vec<&str> = seen.answered.iter().map(|a| a.0.as_str()).collect();
        assert_eq!(answered, ["item 0", "item 1", "item 2", "item 3"]);
    }

    #[tokio::test(start_paused = true)]
    async fn refusals_of_requests_sent_before_a_pause_make_no_pause_of_their_own() {
        // Every first answer is "too many requests": three were on their way.
        let (end, seen) = try_run(
            names(3),
            Pace { every: Duration::from_millis(100), at_once: 3 },
            Duration::from_secs(1),
            |_, before| if before == 0 { Answer::Busy } else { Answer::NotOnMarket },
            None,
        )
        .await;
        assert_eq!(end, RunEnd::Done);
        assert_eq!(seen.pauses.len(), 1);
        // Asked again after the pause, in the order they were in.
        let again: Vec<&str> = seen.starts[3..].iter().map(|s| s.0.as_str()).collect();
        assert_eq!(again, ["item 0", "item 1", "item 2"]);
        assert_eq!(seen.answered.len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_gives_up_when_steam_keeps_refusing() {
        let (end, seen) = try_run(names(5), PACE, Duration::from_millis(300), |_, _| Answer::Busy, None).await;
        assert_eq!(end, RunEnd::GaveUp { left: 5 });
        // 20 + 40 + 80 + 160 s of pauses, each one longer, the pace slower.
        let pauses: Vec<u64> = seen.pauses.iter().map(|p| p.0.as_secs()).collect();
        assert_eq!(pauses, [20, 40, 80, 160]);
        assert!(seen.pauses.windows(2).all(|w| w[1].1 >= w[0].1));
        assert!(seen.answered.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn trouble_is_asked_again_later_then_given_up_on() {
        // item 0 always has trouble; the others are fine.
        let (end, seen) = try_run(
            names(3),
            Pace { every: Duration::from_secs(1), at_once: 1 },
            Duration::from_millis(100),
            |name, _| if name == "item 0" { Answer::Trouble("HTTP 502".into()) } else { Answer::Book(book(9)) },
            None,
        )
        .await;
        assert_eq!(end, RunEnd::Done);
        let order: Vec<&str> = seen.starts.iter().map(|s| s.0.as_str()).collect();
        // Asked again after the others, three times in all.
        assert_eq!(order, ["item 0", "item 1", "item 2", "item 0", "item 0"]);
        assert_eq!(seen.answered.last(), Some(&("item 0".into(), Answer::Failed("HTTP 502".into()))));
        assert!(seen.pauses.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_stopped_run_keeps_the_answers_on_their_way_and_asks_no_more() {
        let (end, seen) = try_run(
            names(10),
            Pace { every: Duration::from_millis(500), at_once: 3 },
            Duration::from_millis(1200),
            |_, _| Answer::Book(book(5)),
            Some(1),
        )
        .await;
        assert_eq!(end, RunEnd::Stopped);
        // The first answer comes at 1.2 s; by then three requests had
        // started (0, 0.5, 1.0), whose answers still count.
        assert_eq!(seen.starts.len(), 3);
        assert_eq!(seen.answered.len(), 3);
    }

    #[test]
    fn prices_are_fresh_for_an_hour_and_kept_for_a_week() {
        let now = 1_764_244_800_000;
        let seen = |ago: i64| Seen { lowest_sell: Some(21), highest_buy: Some(14), sell_listings: 1, checked_at: now - ago };
        assert!(seen(59 * 60_000).fresh(now));
        assert!(!seen(61 * 60_000).fresh(now));
        // A time well ahead of the clock was a wrong clock.
        assert!(!seen(-10 * 60_000).fresh(now));
        assert!(seen(-30_000).fresh(now));

        let dir = std::env::temp_dir().join(format!("idlekit-prices-{}", std::process::id()));
        let file = dir.join(FILE);
        let mut list = PriceList::default();
        list.put(31, "774361-Our Lady of the Charred Visage", seen(10 * 60_000));
        list.put(31, "old", seen(8 * 24 * 3_600_000));
        list.put(1, "in dollars", seen(0));
        list.put_wallet("76561197960265733", Wallet::standard(31, false), now);
        PriceList::save(&list.to_bytes(), &file).unwrap();
        let back = PriceList::load(&file, now);
        assert_eq!(back.get(31, "774361-Our Lady of the Charred Visage"), Some(seen(10 * 60_000)));
        assert_eq!(back.get(31, "old"), None);
        // By currency: a price in dollars is no price in riyals.
        assert_eq!(back.get(31, "in dollars"), None);
        assert_eq!(back.get(1, "in dollars"), Some(seen(0)));
        assert_eq!(back.wallet("76561197960265733").map(|w| w.wallet.currency), Some(31));
        assert!(back.wallet("76561197960265733").unwrap().fresh(now + 23 * 3_600_000));
        assert!(!back.wallet("76561197960265733").unwrap().fresh(now + 25 * 3_600_000));
        std::fs::remove_dir_all(&dir).unwrap();
        // A file that can't be read is an empty list.
        assert_eq!(PriceList::load(&dir.join("nothing.json"), now), PriceList::default());
    }
}
