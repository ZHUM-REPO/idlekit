//! Pricing a whole inventory without upsetting Steam's market. Requests are
//! paced by when they start, with a few waiting for their answer at once, so
//! one slow answer doesn't hold up the rest; "too many requests" pauses all
//! of them and slows the pace; and what was learned is kept on disk for an
//! hour, so opening the market again doesn't ask Steam again.
//!
//! Prices come in two rounds. The market's search answers for a hundred
//! items at a time, so a few dozen requests give every item's cheapest
//! listing; then each item's order book, one request an item, gives its buy
//! orders too.
use super::market::{Book, SearchFor, SearchKind, Wallet, SEARCH_PAGE};
use futures_util::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
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
/// Buy orders read this long ago still stand beside a newer cheapest listing.
const OFFERS_KEPT_FOR_MS: i64 = 24 * 60 * 60 * 1000;

// ------------------------------------------------------------------ kept --

/// An item's prices as last seen.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
    pub lowest_sell: Option<u32>,
    pub highest_buy: Option<u32>,
    pub sell_listings: u32,
    /// When Steam was asked for the cheapest listing, in milliseconds since 1970.
    pub checked_at: i64,
    /// When the buy orders were read, which takes the item's order book: 0
    /// while only the market's search has answered. Lists written before
    /// there was a search don't say: every price in them is an order book's.
    #[serde(default = "not_said")]
    pub offers_at: i64,
}

fn not_said() -> i64 {
    NOT_SAID
}

const NOT_SAID: i64 = -1;

impl Seen {
    /// From the item's order book: all there is to know.
    pub fn of(book: &Book, checked_at: i64) -> Seen {
        Seen {
            lowest_sell: book.lowest_sell,
            highest_buy: book.highest_buy,
            sell_listings: book.sell_listings,
            checked_at,
            offers_at: checked_at,
        }
    }

    /// From the market's search, which tells the cheapest listing and how
    /// many there are, and nothing of buy orders. Those stay as an order
    /// book read in the last day had them (`before`), unless the cheapest
    /// listing is now at or under the highest offer of back then: the book
    /// has moved, and that offer is nothing to go by any more.
    pub fn of_search(lowest_sell: u32, sell_listings: u32, checked_at: i64, before: Option<&Seen>) -> Seen {
        let offers = before
            .filter(|b| b.offers_at > 0 && is_recent(b.offers_at, checked_at, OFFERS_KEPT_FOR_MS))
            .filter(|b| b.highest_buy.is_none_or(|offer| offer < lowest_sell));
        Seen {
            lowest_sell: Some(lowest_sell),
            highest_buy: offers.and_then(|b| b.highest_buy),
            sell_listings,
            checked_at,
            offers_at: offers.map_or(0, |b| b.offers_at),
        }
    }

    /// The cheapest listing is recent enough not to ask Steam again.
    pub fn fresh(&self, now: i64) -> bool {
        is_recent(self.checked_at, now, FRESH_FOR_MS)
    }

    /// The buy orders are recent enough not to read the order book again.
    pub fn offers_fresh(&self, now: i64) -> bool {
        self.offers_at > 0 && is_recent(self.offers_at, now, FRESH_FOR_MS)
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
            // A price from before the market's search was asked: an order book's.
            for seen in books.values_mut().filter(|seen| seen.offers_at == NOT_SAID) {
                seen.offers_at = seen.checked_at;
            }
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

// -------------------------------------------------------------- searches --

/// Games in one search, at most.
const GAMES_PER_SEARCH: usize = 30;
/// Fewer items than this are as quickly asked about one by one.
const FEWEST_FOR_SEARCHES: usize = 12;
/// A search with more results a game than this didn't keep to its games.
const MOST_A_GAME: usize = 80;

/// The game a Steam item's market name starts with ("440-PYRO (Trading
/// Card)" is one of Team Fortress 2's); `None` when it names none.
pub fn game_of(hash_name: &str) -> Option<u32> {
    let (id, rest) = hash_name.split_once('-')?;
    if rest.is_empty() || id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    id.parse().ok().filter(|&id| id > 0)
}

/// The searches that tell the cheapest listing of many items at once.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    /// In the order the items came: the first search has the first items' games.
    pub searches: Vec<SearchFor>,
    /// How many of the items the searches ask about. The rest have no game
    /// in their name, or a class the search isn't asked for.
    pub covered: usize,
}

impl Plan {
    /// For these items, each with its class in the inventory when that is known.
    pub fn of<'a>(items: impl IntoIterator<Item = (&'a str, Option<&'a str>)>) -> Plan {
        // Each kind's games, as their items come, and where the first one was.
        let mut games: BTreeMap<SearchKind, Vec<(u32, usize)>> = BTreeMap::new();
        let mut known: HashSet<(SearchKind, u32)> = HashSet::new();
        let mut covered = 0;
        for (at, (name, class)) in items.into_iter().enumerate() {
            let (Some(kind), Some(game)) = (class.and_then(SearchKind::of_class), game_of(name)) else { continue };
            covered += 1;
            if known.insert((kind, game)) {
                games.entry(kind).or_default().push((game, at));
            }
        }
        let mut searches: Vec<(usize, SearchFor)> = Vec::new();
        for (kind, games) in games {
            for part in games.chunks(GAMES_PER_SEARCH) {
                searches.push((part[0].1, SearchFor { kind, games: part.iter().map(|g| g.0).collect() }));
            }
        }
        searches.sort_by_key(|s| s.0);
        Plan { searches: searches.into_iter().map(|s| s.1).collect(), covered }
    }

    /// How many requests the searches take, about: a game has some eight
    /// cards on the market, as many foil ones, and a dozen other items.
    pub fn requests(&self) -> usize {
        self.searches
            .iter()
            .map(|s| {
                let a_game = if s.kind == SearchKind::Others { 13 } else { 8 };
                (s.games.len() * a_game).div_ceil(SEARCH_PAGE).max(1)
            })
            .sum()
    }

    /// How many games the searches name.
    pub fn games(&self) -> usize {
        self.searches.iter().flat_map(|s| &s.games).collect::<HashSet<_>>().len()
    }

    /// Whether searching is the faster way to these items' prices: a game's
    /// items come in one answer, so it is from a dozen items up.
    pub fn worth_it(&self) -> bool {
        self.covered >= FEWEST_FOR_SEARCHES
    }
}

/// Where a search's further answers start, when its first one held `first`
/// items of `total`. `None` when the search has far more items than its
/// games can have: Steam didn't keep to what was asked.
pub fn further_starts(what: &SearchFor, total: usize, first: usize) -> Option<Vec<usize>> {
    if total > what.games.len() * MOST_A_GAME {
        return None;
    }
    if first == 0 || first >= total {
        return Some(Vec::new());
    }
    Some((first..total).step_by(first).collect())
}

// ---------------------------------------------------------------- asking --

/// What asking Steam for one thing came to: an item's order book, or one
/// answer of the market's search.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer<T = Book> {
    Got(T),
    /// Steam has no market for this item.
    NotOnMarket,
    /// "Too many requests": every request waits, and this one is sent again.
    Busy,
    /// Steam, or the connection, had trouble: this one is sent again later.
    Trouble(String),
    /// There is no answer to be had, and why.
    Failed(String),
}

/// How fast to ask, and for how long against refusals.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    /// From the start of one request to the start of the next.
    pub every: Duration,
    /// Requests waiting for their answer at the same time, at most.
    pub at_once: usize,
    /// Pauses in a row, with no answer in between, before the run gives up.
    pub patience: u32,
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
/// Pauses in a row, with no answer in between, before a run of order
/// books gives up; and before the market's search is given up on, which
/// has the order books to fall back on.
pub const PAUSES_BEFORE_GIVING_UP: u32 = 4;
pub const PAUSES_BEFORE_NO_SEARCH: u32 = 2;
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
    /// Steam kept saying "too many requests", or kept having trouble
    /// answering (`cause`); this many items weren't priced.
    GaveUp { left: usize, cause: Cause },
}

/// Why a run pauses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// Steam answered "too many requests" (HTTP 429).
    TooMany,
    /// Steam had trouble answering (a 5xx, or no answer at all), several times in a row.
    Trouble,
}

/// What a run tells its caller as it goes.
#[derive(Debug, PartialEq)]
pub enum News<'a, T = Book> {
    /// An item's last word: a book, not on the market, or failed.
    Answered(&'a str, Answer<T>),
    /// An item Steam had trouble with goes to the back of the line, to be
    /// asked again: why, and which try comes next.
    Again(&'a str, &'a str, u8),
    /// A pause: how long it is, the pace after it, and why.
    Pausing { pause: Duration, every: Duration, cause: Cause },
}

/// Asks about every item in `names`, first to last, at `pace`, and tells
/// `news` each item's answer as it comes. `stop` is looked at often; once it
/// says so, the run ends, after the answers already on their way.
pub async fn run<A, Fut, T>(
    names: Vec<String>,
    pace: Pace,
    pacer: &mut Pacer,
    stop: impl Fn() -> bool,
    ask: A,
    mut news: impl FnMut(News<'_, T>),
) -> RunEnd
where
    A: Fn(String) -> Fut,
    Fut: Future<Output = Answer<T>>,
{
    let at_once = pace.at_once.max(1);
    let patience = pace.patience.max(1);
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
                    let got: Option<(String, u8, Instant, Answer<T>)> = asking.next().await;
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
                if pauses > patience {
                    return RunEnd::GaveUp { left: queue.len() + asking.len(), cause: Cause::TooMany };
                }
                let pause = take_a_pause(pacer, &mut every);
                pause_began = Some(Instant::now());
                news(News::Pausing { pause, every, cause: Cause::TooMany });
            }
            Answer::Trouble(why) => {
                if tries + 1 < TRIES {
                    news(News::Again(&name, &why, tries + 2));
                    queue.push_back((name, tries + 1));
                } else {
                    news(News::Answered(&name, Answer::Failed(why)));
                }
                troubles += 1;
                if troubles >= TROUBLES_FOR_A_PAUSE && new_pause {
                    troubles = 0;
                    refused = 0;
                    pauses += 1;
                    if pauses > patience {
                        return RunEnd::GaveUp { left: queue.len() + asking.len(), cause: Cause::Trouble };
                    }
                    let pause = take_a_pause(pacer, &mut every);
                    pause_began = Some(Instant::now());
                    news(News::Pausing { pause, every, cause: Cause::Trouble });
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
                News::Pausing { pause, every, .. } => seen.borrow_mut().pauses.push((pause, every)),
                News::Again(..) => {}
            },
        )
        .await;
        let seen = Rc::try_unwrap(seen).ok().expect("the run is over").into_inner();
        (end, seen)
    }

    const PACE: Pace = Pace { every: Duration::from_millis(1500), at_once: 3, patience: PAUSES_BEFORE_GIVING_UP };

    #[tokio::test(start_paused = true)]
    async fn requests_start_a_pace_apart_however_long_answers_take() {
        // Answers that take longer than the pace: the next request starts anyway.
        let (end, seen) = try_run(names(6), PACE, Duration::from_millis(4000), |_, _| Answer::Got(book(21)), None).await;
        assert_eq!(end, RunEnd::Done);
        let starts: Vec<f64> = seen.starts.iter().map(|s| s.1).collect();
        // 0, 1.5, 3: three waiting. The first answer comes at 4: the fourth
        // starts then, and so on, never closer than the pace.
        assert_eq!(starts, vec![0.0, 1.5, 3.0, 4.5, 6.0, 7.5]);
        assert_eq!(seen.most_at_once, 3);
        assert_eq!(seen.answered.len(), 6);
        assert_eq!(seen.answered[0], ("item 0".into(), Answer::Got(book(21))));
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
            Pace { every: Duration::from_secs(1), at_once: 1, patience: PAUSES_BEFORE_GIVING_UP },
            Duration::from_millis(200),
            |name, before| if name == "item 1" && before == 0 { Answer::Busy } else { Answer::Got(book(30)) },
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
            Pace { every: Duration::from_millis(100), at_once: 3, patience: PAUSES_BEFORE_GIVING_UP },
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
        assert_eq!(end, RunEnd::GaveUp { left: 5, cause: Cause::TooMany });
        // 20 + 40 + 80 + 160 s of pauses, each one longer, the pace slower.
        let pauses: Vec<u64> = seen.pauses.iter().map(|p| p.0.as_secs()).collect();
        assert_eq!(pauses, [20, 40, 80, 160]);
        assert!(seen.pauses.windows(2).all(|w| w[1].1 >= w[0].1));
        assert!(seen.answered.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_with_less_patience_gives_up_sooner() {
        // The market's search: two pauses, and the order books take over.
        let pace = Pace { patience: PAUSES_BEFORE_NO_SEARCH, ..PACE };
        let (end, seen) = try_run(names(3), pace, Duration::from_millis(300), |_, _| Answer::Busy, None).await;
        assert_eq!(end, RunEnd::GaveUp { left: 3, cause: Cause::TooMany });
        let pauses: Vec<u64> = seen.pauses.iter().map(|p| p.0.as_secs()).collect();
        assert_eq!(pauses, [20, 40]);
        // One answer in between, and it has all its patience again.
        let (end, seen) = try_run(
            names(2),
            Pace { every: Duration::from_secs(1), at_once: 1, patience: PAUSES_BEFORE_NO_SEARCH },
            Duration::from_millis(100),
            |_, before| if before < 2 { Answer::Busy } else { Answer::NotOnMarket },
            None,
        )
        .await;
        assert_eq!(end, RunEnd::Done);
        assert_eq!(seen.pauses.len(), 4);
        assert_eq!(seen.answered.len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn trouble_is_asked_again_later_then_given_up_on() {
        // item 0 always has trouble; the others are fine.
        let (end, seen) = try_run(
            names(3),
            Pace { every: Duration::from_secs(1), at_once: 1, patience: PAUSES_BEFORE_GIVING_UP },
            Duration::from_millis(100),
            |name, _| if name == "item 0" { Answer::Trouble("HTTP 502".into()) } else { Answer::Got(book(9)) },
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
            Pace { every: Duration::from_millis(500), at_once: 3, patience: PAUSES_BEFORE_GIVING_UP },
            Duration::from_millis(1200),
            |_, _| Answer::Got(book(5)),
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
        let seen =
            |ago: i64| Seen { lowest_sell: Some(21), highest_buy: Some(14), sell_listings: 1, checked_at: now - ago, offers_at: now - ago };
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

    const NOW: i64 = 1_764_244_800_000;
    const MINUTE: i64 = 60_000;

    #[test]
    fn a_search_gives_the_cheapest_listing_and_keeps_the_buy_orders_read_before() {
        let read = |ago: i64, offer: Option<u32>| Seen::of(
            &Book { currency: 31, lowest_sell: Some(21), highest_buy: offer, sell_listings: 9 },
            NOW - ago,
        );
        // Nothing known before: a price, and no buy orders yet.
        let first = Seen::of_search(20, 300, NOW, None);
        assert_eq!(first, Seen { lowest_sell: Some(20), highest_buy: None, sell_listings: 300, checked_at: NOW, offers_at: 0 });
        assert!(first.fresh(NOW + 59 * MINUTE));
        assert!(!first.offers_fresh(NOW));

        // An order book read two hours ago: its offers stand, and are due again.
        let book = read(120 * MINUTE, Some(14));
        let after = Seen::of_search(19, 310, NOW, Some(&book));
        assert_eq!((after.lowest_sell, after.highest_buy, after.sell_listings), (Some(19), Some(14), 310));
        assert_eq!((after.checked_at, after.offers_at), (NOW, NOW - 120 * MINUTE));
        assert!(after.fresh(NOW) && !after.offers_fresh(NOW));
        // Read ten minutes ago, they aren't due.
        assert!(Seen::of_search(19, 310, NOW, Some(&read(10 * MINUTE, Some(14)))).offers_fresh(NOW));
        // "Nobody is buying" is something read too.
        let nobody = Seen::of_search(19, 310, NOW, Some(&read(10 * MINUTE, None)));
        assert_eq!((nobody.highest_buy, nobody.offers_at), (None, NOW - 10 * MINUTE));

        // The cheapest listing is at or under that offer now: the book has moved.
        for lowest in [14, 9] {
            let moved = Seen::of_search(lowest, 5, NOW, Some(&book));
            assert_eq!((moved.highest_buy, moved.offers_at), (None, 0), "{lowest}");
        }
        // Offers older than a day are nothing to go by, nor are a search's own.
        assert_eq!(Seen::of_search(30, 5, NOW, Some(&read(25 * 60 * MINUTE, Some(14)))).offers_at, 0);
        assert_eq!(Seen::of_search(30, 5, NOW, Some(&first)).offers_at, 0);
        // An order book has it all.
        assert!(book.offers_fresh(NOW - 61 * MINUTE) && !book.offers_fresh(NOW));
    }

    #[test]
    fn a_list_from_before_the_search_holds_order_books() {
        let dir = std::env::temp_dir().join(format!("idlekit-prices-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(FILE);
        // As 2.3 wrote it: no word of when the buy orders were read.
        let then = NOW - 10 * MINUTE;
        let text = format!(
            r#"{{"books":{{"31":{{"1-A":{{"lowestSell":21,"highestBuy":14,"sellListings":3,"checkedAt":{then}}},"1-B":{{"lowestSell":20,"highestBuy":null,"sellListings":300,"checkedAt":{then},"offersAt":0}}}}}},"wallets":{{}}}}"#
        );
        std::fs::write(&file, text).unwrap();
        let list = PriceList::load(&file, NOW);
        let a = list.get(31, "1-A").unwrap();
        assert_eq!(a.offers_at, then);
        assert!(a.offers_fresh(NOW));
        // One the search priced stays without buy orders.
        let b = list.get(31, "1-B").unwrap();
        assert_eq!(b.offers_at, 0);
        assert!(b.fresh(NOW) && !b.offers_fresh(NOW));
        // Written back, it says so.
        PriceList::save(&list.to_bytes(), &file).unwrap();
        assert_eq!(PriceList::load(&file, NOW), list);
        assert!(std::fs::read_to_string(&file).unwrap().contains(&format!(r#""offersAt":{then}"#)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn searches_cover_an_inventory_by_game_and_kind() {
        assert_eq!(game_of("440-PYRO (Trading Card)"), Some(440));
        assert_eq!(game_of("1578650-:sleeperengage:"), Some(1578650));
        assert_eq!(game_of("753-Sack of Gems"), Some(753));
        for none in ["Sack of Gems", "-A", "440-", "4x0-A", "0-A", "99999999999-A", ""] {
            assert_eq!(game_of(none), None, "{none}");
        }

        let items = [
            ("20-Card B", Some("card")),
            ("10-Card A", Some("card")),
            ("20-:smile:", Some("emoticon")),
            ("20-Card C", Some("card")),
            ("10-Card A (Foil)", Some("foil")),
            ("30-Sunset", Some("background")),
            ("30-Booster Pack", Some("booster")),
            ("753-Sack of Gems", Some("gems")),
            ("40-Something", Some("other")),
            ("50-Card Z", None),
            ("No game", Some("card")),
        ];
        let plan = Plan::of(items);
        assert_eq!(
            plan.searches,
            vec![
                SearchFor { kind: SearchKind::Cards, games: vec![20, 10] },
                SearchFor { kind: SearchKind::Others, games: vec![20, 30] },
                SearchFor { kind: SearchKind::Foils, games: vec![10] },
            ]
        );
        assert_eq!((plan.covered, plan.games(), plan.requests()), (7, 3, 3));
        // Seven items: their order books are as quick.
        assert!(!plan.worth_it());
        assert_eq!(Plan::of([]), Plan::default());
        assert!(!Plan::default().worth_it());

        // 645 cards of 130 games: five searches of thirty games or fewer, in
        // the order the games came, and a few answers each.
        let names: Vec<String> = (0..645).map(|k| format!("{}-Card {k}", 1000 + k % 130)).collect();
        let plan = Plan::of(names.iter().map(|n| (n.as_str(), Some("card"))));
        let sizes: Vec<usize> = plan.searches.iter().map(|s| s.games.len()).collect();
        assert_eq!(sizes, [30, 30, 30, 30, 10]);
        assert_eq!(plan.searches[0].games[..3], [1000, 1001, 1002]);
        assert_eq!(plan.searches[4].games[9], 1129);
        assert_eq!((plan.covered, plan.games(), plan.requests()), (645, 130, 13));
        assert!(plan.worth_it());
        // A dozen cards of one game are worth a search; eleven aren't.
        let of_one_game = |n: usize| Plan::of((0..n).map(|_| ("7-Card", Some("card"))));
        assert!(of_one_game(12).worth_it());
        assert!(!of_one_game(11).worth_it());
        // Twenty emoticons of twenty games: three answers, by the dozen or so
        // other items a game has.
        let spread: Vec<String> = (0..20).map(|k| format!("{}-:e:", 100 + k)).collect();
        let plan = Plan::of(spread.iter().map(|n| (n.as_str(), Some("emoticon"))));
        assert_eq!((plan.searches.len(), plan.requests(), plan.covered), (1, 3, 20));
        // Kinds and games mixed: each kind's games in searches of its own.
        let mixed: Vec<String> = (0..12).map(|k| format!("{}-:e:", 100 + k % 4)).chain((0..40).map(|k| format!("{}-C", 200 + k))).collect();
        let plan = Plan::of(mixed.iter().enumerate().map(|(k, n)| (n.as_str(), Some(if k < 12 { "emoticon" } else { "card" }))));
        assert_eq!((plan.searches.len(), plan.requests(), plan.covered), (3, 5, 52));
        assert!(plan.worth_it());
    }

    #[test]
    fn a_search_goes_on_where_its_first_answer_ended() {
        let what = SearchFor { kind: SearchKind::Cards, games: (1..=30).collect() };
        assert_eq!(further_starts(&what, 412, 100), Some(vec![100, 200, 300, 400]));
        assert_eq!(further_starts(&what, 100, 100), Some(vec![]));
        assert_eq!(further_starts(&what, 37, 37), Some(vec![]));
        assert_eq!(further_starts(&what, 0, 0), Some(vec![]));
        // Steam gave fewer at a time than asked for: on from there, as many at a time.
        assert_eq!(further_starts(&what, 100, 30), Some(vec![30, 60, 90]));
        // Nothing in the answer, though it says there is more: no going on.
        assert_eq!(further_starts(&what, 50, 0), Some(vec![]));
        // Far more than thirty games have: the search didn't keep to them.
        assert_eq!(further_starts(&what, 189_268, 100), None);
        assert_eq!(further_starts(&what, 2400, 100).map(|s| s.len()), Some(23));
        assert_eq!(further_starts(&what, 2401, 100), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_run_paces_search_answers_like_order_books() {
        // The same pacing whatever an answer holds: here, numbers.
        let mut pacer = Pacer::default();
        let began = Instant::now();
        let mut got: Vec<(String, u64, f64)> = Vec::new();
        let end = run(
            vec!["0:0".to_string(), "1:0".to_string(), "2:0".to_string()],
            Pace { every: Duration::from_secs(1), at_once: 3, patience: PAUSES_BEFORE_GIVING_UP },
            &mut pacer,
            || false,
            |job: String| async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                if job == "1:0" { Answer::Failed("HTTP 400".into()) } else { Answer::Got(job.len() as u64) }
            },
            |news: News<'_, u64>| {
                if let News::Answered(job, Answer::Got(n)) = news {
                    got.push((job.to_string(), n, began.elapsed().as_secs_f64()));
                }
            },
        )
        .await;
        assert_eq!(end, RunEnd::Done);
        // A second apart; the one that failed is no answer.
        let jobs: Vec<(&str, u64)> = got.iter().map(|g| (g.0.as_str(), g.1)).collect();
        assert_eq!(jobs, [("0:0", 3), ("2:0", 3)]);
        assert!((got[0].2 - 0.3).abs() < 1e-6 && (got[1].2 - 2.3).abs() < 1e-6, "{got:?}");
    }
}
