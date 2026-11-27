//! Steam Community Market: inventory (app 753 / context 6 — cards, emoticons,
//! backgrounds, boosters), live order-book pricing, listing and delisting.
use crate::net::Logged;
use super::community::WebSession;
use anyhow::{anyhow, bail, Context, Result};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct InvItem {
    pub market_hash_name: String,
    pub name: String,
    pub item_type: String,
    /// "card" | "foil" | "emoticon" | "background" | "booster" | "gems" | "other"
    pub class: String,
    pub game: String,
    pub icon_url: String,
    pub marketable: bool,
    pub count: u32,
    pub asset_ids: Vec<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct PriceInfo {
    pub market_hash_name: String,
    /// The currency of every price here (Steam's id for it).
    pub currency: u32,
    /// Prices are what the buyer pays, in the currency's minor units.
    pub lowest_sell: Option<u32>,
    pub highest_buy: Option<u32>,
    pub sell_listings: u32,
    /// Suggested buyer price after strategy and min/max limits.
    pub suggested: Option<u32>,
    /// What you receive after Steam's fees for `suggested`.
    pub you_receive: Option<u32>,
    /// When Steam was asked, in milliseconds since 1970; 0 when it wasn't.
    pub checked_at: i64,
    /// When the buy orders (`highest_buy`) were read, which takes the item's
    /// order book: 0 while only the market's search has answered.
    pub offers_at: i64,
    /// Why there is no price, when Steam couldn't say.
    pub problem: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub listing_id: String,
    pub name: String,
    pub game: String,
    pub price_text: String,
    /// "active" | "pending" (awaiting mobile/email confirmation)
    pub status: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SellOutcome {
    pub asset_id: String,
    pub ok: bool,
    pub needs_confirmation: bool,
    pub message: String,
}

// ----------------------------------------------------------------- fees ---

/// How the market charges in the signed-in account's wallet currency. Steam
/// published these numbers on the market page (`g_rgWalletInfo`) and its own
/// sell dialog computed from them; the market Steam opened in November 2026
/// only says which currency, and whether its prices are whole units. The
/// defaults are a US-dollar wallet's. Amounts are in the currency's minor units.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Wallet {
    /// Steam's currency id (1 = USD, 3 = EUR, 31 = SAR, …).
    pub currency: u32,
    pub steam_percent: f64,
    pub steam_minimum: u32,
    pub fee_base: u32,
    pub publisher_percent: f64,
    /// Smallest step between two prices.
    pub increment: u32,
    /// Smallest amount a seller can ask for.
    pub market_minimum: u32,
}

impl Default for Wallet {
    fn default() -> Self {
        Wallet {
            currency: 1,
            steam_percent: 0.05,
            steam_minimum: 1,
            fee_base: 0,
            publisher_percent: 0.10,
            increment: 1,
            market_minimum: 1,
        }
    }
}

impl Wallet {
    /// Reads the wallet block of the market page; `None` when the page has
    /// none (signed out). A number that is missing or implausible keeps its
    /// US-dollar default.
    pub fn parse(html: &str) -> Option<Wallet> {
        // Every wallet number in one pass over the page, with one pattern.
        let mut numbers: BTreeMap<&str, f64> = BTreeMap::new();
        for found in re!(r#""(wallet_[a-z_]+)"\s*:\s*"?([0-9]+(?:\.[0-9]+)?)"?"#).captures_iter(html) {
            if let (Some(name), Ok(value)) = (found.get(1), found[2].parse::<f64>()) {
                // The first one counts, as when each was looked for by name.
                numbers.entry(name.as_str()).or_insert(value);
            }
        }
        Wallet::from_fields(|name| numbers.get(name).copied())
    }

    /// A wallet from Steam's `wallet_…` numbers, wherever they were found.
    fn from_fields(field: impl Fn(&str) -> Option<f64>) -> Option<Wallet> {
        let whole = |name: &str, low: f64, high: f64| field(name).filter(|v| (low..=high).contains(v)).map(|v| v as u32);
        let share = |name: &str| field(name).filter(|v| (0.0..0.5).contains(v));
        let d = Wallet::default();
        Some(Wallet {
            currency: whole("wallet_currency", 1.0, 1000.0)?,
            steam_percent: share("wallet_fee_percent").unwrap_or(d.steam_percent),
            steam_minimum: whole("wallet_fee_minimum", 1.0, 1e7).unwrap_or(d.steam_minimum),
            fee_base: whole("wallet_fee_base", 0.0, 1e7).unwrap_or(d.fee_base),
            publisher_percent: share("wallet_publisher_fee_percent_default").unwrap_or(d.publisher_percent),
            increment: whole("wallet_currency_increment", 1.0, 1e7).unwrap_or(d.increment),
            market_minimum: whole("wallet_market_minimum", 1.0, 1e7).unwrap_or(d.market_minimum),
        })
    }

    /// Steam's standard fees, in a currency. Where prices are whole units
    /// only, every price is a whole number of them.
    pub fn standard(currency: u32, whole_units: bool) -> Wallet {
        let step = if whole_units { 100 } else { 1 };
        Wallet { currency, increment: step, market_minimum: step, ..Wallet::default() }
    }

    /// Reads the wallet off the market page Steam opened in November 2026:
    /// the page's data says which currency its prices are in (the wallet's,
    /// when signed in) and whether they are whole units; the fees are
    /// Steam's standard ones, unless the data has the old wallet numbers.
    pub fn from_market_app(html: &str) -> Option<Wallet> {
        let data = page_data(html)?;
        let old = find_in(&data, &|map| {
            map.contains_key("wallet_currency").then(|| {
                Wallet::from_fields(|name| {
                    let v = map.get(name)?;
                    v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
                })
            })?
        });
        old.or_else(|| {
            find_in(&data, &|map| {
                let currency = map.get("eCurrency")?.as_u64().filter(|c| (1..=1000).contains(c))?;
                let whole_units = map.get("bWholeUnitsOnly")?.as_bool()?;
                Some(Wallet::standard(currency as u32, whole_units))
            })
        })
    }

    /// The wallet, from either kind of market page.
    pub fn read(html: &str) -> Option<Wallet> {
        Wallet::parse(html).or_else(|| Wallet::from_market_app(html))
    }

    /// What a buyer pays for the seller to receive this much: Steam's fee
    /// (a share with a minimum, plus a base) and the game's fee (a share,
    /// at least 1), each rounded down, the way Steam's sell dialog does it.
    pub fn buyer_pays(&self, seller_receives: u32) -> u32 {
        if seller_receives == 0 {
            return 0;
        }
        let amount = seller_receives as f64;
        let steam_fee = ((amount * self.steam_percent).max(self.steam_minimum as f64) + self.fee_base as f64).floor() as u32;
        let publisher_fee =
            if self.publisher_percent > 0.0 { (amount * self.publisher_percent).max(1.0).floor() as u32 } else { 0 };
        seller_receives + steam_fee + publisher_fee
    }

    /// Largest amount the seller can receive so the buyer pays at most `buyer_price`.
    pub fn seller_receives(&self, buyer_price: u32) -> u32 {
        if buyer_price < self.lowest_price() {
            return 0;
        }
        let rate = 1.0 + self.steam_percent + self.publisher_percent;
        let mut s = ((buyer_price.saturating_sub(self.fee_base) as f64) / rate).floor() as u32 + 2;
        while s > 0 && self.buyer_pays(s) > buyer_price {
            s -= 1;
        }
        s
    }

    /// The lowest price a buyer can be asked to pay.
    pub fn lowest_price(&self) -> u32 {
        self.buyer_pays(self.market_minimum.max(1))
    }

    /// A price step: at least one increment, and a whole number of them.
    fn step(&self, cents: u32) -> u32 {
        let inc = self.increment.max(1);
        cents.max(1).div_ceil(inc) * inc
    }
}

pub fn suggest_price(
    wallet: &Wallet,
    lowest_sell: Option<u32>,
    highest_buy: Option<u32>,
    strategy: &str,
    undercut: u32,
    min_cents: u32,
    max_cents: u32,
) -> Option<u32> {
    let floor = wallet.lowest_price();
    let base = match strategy {
        "matchLowest" => lowest_sell,
        "aboveHighestBuy" => highest_buy.map(|b| b + wallet.step(undercut)),
        _ => lowest_sell.map(|l| if undercut == 0 { l } else { l.saturating_sub(wallet.step(undercut)) }),
    }
    .or(lowest_sell)
    .or_else(|| highest_buy.map(|b| b + wallet.step(1)))?;
    let mut p = base.max(floor);
    if min_cents > 0 {
        p = p.max(min_cents);
    }
    if max_cents > 0 {
        p = p.min(max_cents);
    }
    // Never undercut below what a buyer is already offering.
    if let Some(b) = highest_buy {
        if strategy != "aboveHighestBuy" && p <= b {
            p = b + wallet.step(1);
        }
    }
    Some(p.max(floor))
}

/// The data Steam's newer pages arrive with: the JSON in their
/// `valve-ssr-data` script.
fn page_data(html: &str) -> Option<Value> {
    let found = re!(r#"(?s)<script[^>]*\bid="valve-ssr-data"[^>]*>(.*?)</script>"#).captures(html)?;
    serde_json::from_str(found[1].trim()).ok()
}

/// The first object in `v`, depth first, that `pick` takes something from.
/// Parts of the page data are JSON written into a string: those are opened
/// and searched too.
fn find_in<T>(v: &Value, pick: &impl Fn(&serde_json::Map<String, Value>) -> Option<T>) -> Option<T> {
    fn walk<T>(v: &Value, pick: &impl Fn(&serde_json::Map<String, Value>) -> Option<T>, depth: usize) -> Option<T> {
        if depth > 16 {
            return None;
        }
        match v {
            Value::Object(map) => pick(map).or_else(|| map.values().find_map(|x| walk(x, pick, depth + 1))),
            Value::Array(items) => items.iter().find_map(|x| walk(x, pick, depth + 1)),
            Value::String(s) if s.starts_with('{') || s.starts_with('[') => {
                serde_json::from_str::<Value>(s).ok().and_then(|inner| walk(&inner, pick, depth + 1))
            }
            _ => None,
        }
    }
    walk(v, pick, 0)
}

// ------------------------------------------------------------ inventory ---

fn class_of(desc: &Value) -> String {
    let tags = desc.get("tags").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut item_class = String::new();
    let mut foil = false;
    for t in &tags {
        let cat = t.get("category").and_then(Value::as_str).unwrap_or("");
        let internal = t.get("internal_name").and_then(Value::as_str).unwrap_or("");
        if cat == "item_class" {
            item_class = internal.to_string();
        }
        if cat == "cardborder" && internal == "cardborder_1" {
            foil = true;
        }
    }
    match item_class.as_str() {
        "item_class_2" if foil => "foil",
        "item_class_2" => "card",
        "item_class_3" => "background",
        "item_class_4" => "emoticon",
        "item_class_5" => "booster",
        "item_class_7" => "gems",
        _ => "other",
    }
    .into()
}

fn game_of(desc: &Value) -> String {
    desc.get("tags")
        .and_then(Value::as_array)
        .and_then(|tags| {
            tags.iter().find(|t| t.get("category").and_then(Value::as_str) == Some("Game"))
        })
        .and_then(|t| t.get("localized_tag_name").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

pub fn parse_inventory_page(v: &Value, groups: &mut BTreeMap<String, InvItem>) -> (bool, Option<String>) {
    let descs = v.get("descriptions").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut by_class: std::collections::HashMap<(String, String), &Value> = std::collections::HashMap::new();
    for d in &descs {
        let classid = d.get("classid").and_then(Value::as_str).unwrap_or("").to_string();
        let instanceid = d.get("instanceid").and_then(Value::as_str).unwrap_or("0").to_string();
        by_class.insert((classid, instanceid), d);
    }
    for a in v.get("assets").and_then(Value::as_array).cloned().unwrap_or_default() {
        let classid = a.get("classid").and_then(Value::as_str).unwrap_or("").to_string();
        let instanceid = a.get("instanceid").and_then(Value::as_str).unwrap_or("0").to_string();
        let Some(d) = by_class.get(&(classid, instanceid)) else { continue };
        let hash = d.get("market_hash_name").and_then(Value::as_str).unwrap_or("").to_string();
        if hash.is_empty() {
            continue;
        }
        let asset_id = a.get("assetid").and_then(Value::as_str).unwrap_or("").to_string();
        let entry = groups.entry(hash.clone()).or_insert_with(|| InvItem {
            market_hash_name: hash.clone(),
            name: d.get("name").and_then(Value::as_str).unwrap_or(&hash).to_string(),
            item_type: d.get("type").and_then(Value::as_str).unwrap_or("").to_string(),
            class: class_of(d),
            game: game_of(d),
            icon_url: d
                .get("icon_url")
                .and_then(Value::as_str)
                .map(|i| format!("https://community.cloudflare.steamstatic.com/economy/image/{i}/96fx96f"))
                .unwrap_or_default(),
            marketable: d.get("marketable").and_then(Value::as_i64).unwrap_or(0) == 1,
            ..Default::default()
        });
        entry.count += 1;
        entry.asset_ids.push(asset_id);
    }
    let more = v.get("more_items").and_then(Value::as_i64).unwrap_or(0) == 1;
    let last = v.get("last_assetid").and_then(Value::as_str).map(str::to_string);
    (more, last)
}

pub async fn inventory(ws: &WebSession) -> Result<Vec<InvItem>> {
    let mut groups = BTreeMap::new();
    let mut start: Option<String> = None;
    for _ in 0..20 {
        let mut url = format!(
            "https://steamcommunity.com/inventory/{}/753/6?l=english&count=2000",
            ws.steam_id
        );
        if let Some(s) = &start {
            url.push_str(&format!("&start_assetid={s}"));
        }
        let text = ws.get_text(&url).await?;
        let v: Value = serde_json::from_str(&text).context("inventory JSON")?;
        if v.is_null() {
            bail!("Steam returned no inventory (is it private?)");
        }
        let (more, last) = parse_inventory_page(&v, &mut groups);
        if !more || last.is_none() {
            break;
        }
        start = last;
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    let mut items: Vec<InvItem> = groups.into_values().collect();
    items.sort_by(|a, b| b.marketable.cmp(&a.marketable).then(a.game.cmp(&b.game)).then(a.name.cmp(&b.name)));
    Ok(items)
}

// -------------------------------------------------------------- pricing ---

/// Steam's own app id: trading cards, emoticons, backgrounds and booster
/// packs are all its items on the market.
const STEAM_APP: u32 = 753;

pub async fn wallet(ws: &WebSession) -> Result<Wallet> {
    let html = ws.get_text("https://steamcommunity.com/market/").await?;
    Wallet::read(&html).ok_or_else(|| anyhow!("Steam's market page showed no wallet"))
}

/// An item's order book: the cheapest listing and the best standing offer
/// (what a buyer pays, in `currency`'s minor units), and how many listings
/// there are.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub currency: u32,
    pub lowest_sell: Option<u32>,
    pub highest_buy: Option<u32>,
    pub sell_listings: u32,
}

fn cents(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64().and_then(|x| u32::try_from(x).ok()),
        _ => None,
    }
    .filter(|&c| c > 0)
}

/// Reads the answer of the order book lookup Steam's listing pages use since
/// November 2026. `None` when Steam has no market for the item.
pub fn parse_order_book(text: &str) -> Result<Option<Book>> {
    // What doesn't read as JSON is usually a web page instead (an error, a sign-in).
    let v: Value = serde_json::from_str(text).map_err(|_| anyhow!("Steam answered with a page instead of the order book"))?;
    // It comes wrapped: {"data": {"success": true, "data": {the book}}}.
    let answer = if v.get("success").is_some() { &v } else { v.get("data").unwrap_or(&Value::Null) };
    match answer.get("success").and_then(Value::as_bool) {
        Some(true) => {}
        Some(false) => return Ok(None),
        None => bail!("Steam's order book answer isn't in the expected form"),
    }
    let book = answer
        .get("data")
        .filter(|d| d.is_object())
        .ok_or_else(|| anyhow!("Steam's order book answer has no book in it"))?;
    let currency = book
        .get("eCurrency")
        .and_then(Value::as_u64)
        .filter(|c| (1..=1000).contains(c))
        .ok_or_else(|| anyhow!("Steam's order book doesn't say which currency it is in"))?;
    Ok(Some(Book {
        currency: currency as u32,
        lowest_sell: cents(book.get("amtMinSellOrder")),
        highest_buy: cents(book.get("amtMaxBuyOrder")),
        sell_listings: book.get("cSellOrders").and_then(Value::as_u64).map_or(0, |n| n.min(u32::MAX as u64) as u32),
    }))
}

/// An item's order book, looked up by its name, in the currency Steam shows
/// this account. One small answer, where the old way read the item's
/// listing page for a number and then asked for the book by that number.
/// "Too many requests" comes back as a [`SteamBusy`](super::community::SteamBusy) error.
pub async fn order_book_by_name(ws: &WebSession, hash_name: &str) -> Result<Option<Book>> {
    let query = serde_json::to_string(&(STEAM_APP, hash_name))?;
    let url = format!("https://steamcommunity.com/market/orderbook?q=Load&qp={}", urlencoding::encode(&query));
    let text = ws.get_query_or_busy(&url).await.map_err(|e| plain_refusal(e, "order book lookup"))?;
    parse_order_book(&text)
}

/// A refusal says so without the address (long, and the same for every item).
fn plain_refusal(e: anyhow::Error, what: &str) -> anyhow::Error {
    if super::community::steam_busy(&e).is_some() || super::community::connection_failed(&e) {
        e
    } else {
        anyhow!("Steam's {what} answered {}", e.root_cause())
    }
}

/// An item's order book the way Steam's market answered before November
/// 2026, by the number its listing page gave the item (kept from back then),
/// in a given currency.
pub async fn order_book_by_id(ws: &WebSession, item_nameid: u64, currency: u32) -> Result<Book> {
    let url = format!(
        "https://steamcommunity.com/market/itemordershistogram?country=US&language=english&currency={currency}&item_nameid={item_nameid}&two_factor=0&norender=1"
    );
    let text = ws.get_text_or_busy(&url).await?;
    let v: Value = serde_json::from_str(&text).context("order book JSON")?;
    if v.get("success").and_then(Value::as_i64) != Some(1) {
        bail!("Steam refused the price lookup");
    }
    let sell_listings = v
        .get("sell_order_count")
        .or_else(|| v.get("sell_order_summary"))
        .and_then(|c| match c {
            Value::String(s) => s.replace(',', "").parse().ok(),
            Value::Number(n) => n.as_u64().and_then(|x| u32::try_from(x).ok()),
            _ => None,
        })
        .unwrap_or(0);
    Ok(Book {
        currency,
        lowest_sell: cents(v.get("lowest_sell_order")),
        highest_buy: cents(v.get("highest_buy_order")),
        sell_listings,
    })
}

/// A currency as the log names it: its three letters, for the ones Steam's
/// wallets come in.
pub fn currency_code(currency: u32) -> String {
    const CODES: [&str; 42] = [
        "", "USD", "GBP", "EUR", "CHF", "RUB", "PLN", "BRL", "JPY", "NOK", "IDR", "MYR", "PHP", "SGD", "THB", "VND", "KRW",
        "TRY", "UAH", "MXN", "CAD", "AUD", "NZD", "CNY", "INR", "CLP", "PEN", "COP", "ZAR", "HKD", "TWD", "SAR", "AED", "",
        "ARS", "ILS", "", "KZT", "KWD", "QAR", "CRC", "UYU",
    ];
    match CODES.get(currency as usize) {
        Some(code) if !code.is_empty() => code.to_string(),
        _ => format!("currency {currency}"),
    }
}

// ---------------------------------------------------------- market search --

/// The most results Steam's market search gives in one answer.
pub const SEARCH_PAGE: usize = 100;

/// Which of a game's Steam items a search asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SearchKind {
    /// Trading cards, not the foil ones.
    Cards,
    Foils,
    /// Backgrounds, emoticons and booster packs.
    Others,
}

impl SearchKind {
    /// By an inventory item's class ([`InvItem::class`]); `None` for what
    /// the search isn't asked about (gems, and what has no class).
    pub fn of_class(class: &str) -> Option<SearchKind> {
        match class {
            "card" => Some(SearchKind::Cards),
            "foil" => Some(SearchKind::Foils),
            "background" | "emoticon" | "booster" => Some(SearchKind::Others),
            _ => None,
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            SearchKind::Cards => "cards",
            SearchKind::Foils => "foil cards",
            SearchKind::Others => "backgrounds, emoticons and boosters",
        }
    }
}

/// One search: the items of a kind that some games have on the market.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchFor {
    pub kind: SearchKind,
    /// The games' app ids.
    pub games: Vec<u32>,
}

/// What a search says of one item: its cheapest listing (what a buyer pays,
/// in `currency`'s minor units) and how many listings there are. It says
/// nothing of buy orders.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub hash_name: String,
    pub currency: u32,
    /// `None` when the search gave no price that can be gone by.
    pub lowest_sell: Option<u32>,
    pub sell_listings: u32,
}

/// One answer of a search.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SearchPage {
    /// How many items the whole search has.
    pub total: usize,
    /// How many of them this answer held.
    pub results: usize,
    pub found: Vec<Found>,
}

/// The address of a search's answer from `start` on, in name order, so
/// that one answer follows the other without gaps. Steam's own search page
/// asks this way; it names the wallet's currency too.
fn search_url(what: &SearchFor, currency: u32, start: usize) -> String {
    let games: Vec<String> = what.games.iter().map(|g| format!("app_{g}")).collect();
    let filters = match what.kind {
        SearchKind::Cards => json!({ "Game": games, "item_class": ["item_class_2"], "cardborder": ["cardborder_0"] }),
        SearchKind::Foils => json!({ "Game": games, "item_class": ["item_class_2"], "cardborder": ["cardborder_1"] }),
        SearchKind::Others => json!({ "Game": games, "item_class": ["item_class_3", "item_class_4", "item_class_5"] }),
    };
    let query = json!([{
        "appid": STEAM_APP,
        "filters": filters,
        "accessoryFilters": {},
        "price": { "eCurrency": currency },
        "start": start,
        "count": SEARCH_PAGE,
        // By name, first to last.
        "sort": 1,
        "direction": 1,
    }]);
    format!("https://steamcommunity.com/market/actions?q=Search&qp={}", urlencoding::encode(&query.to_string()))
}

/// A price as Steam writes it out ("$0.05", "0,05€", "0.19 SR", "¥ 6",
/// "1.234,50 TL", "5,--€"), in minor units. The last "." or "," with one or
/// two digits after it is the decimal mark; any other separates thousands.
pub fn money_cents(text: &str) -> Option<u32> {
    let first = text.find(|c: char| c.is_ascii_digit())?;
    let last = text.rfind(|c: char| c.is_ascii_digit())?;
    let number = &text[first..=last];
    let mark = number.rfind(['.', ',']);
    let (whole, part) = match mark {
        Some(at) if (1..=2).contains(&(number.len() - at - 1)) && number[at + 1..].bytes().all(|b| b.is_ascii_digit()) => {
            (&number[..at], &number[at + 1..])
        }
        _ => (number, ""),
    };
    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
    let units: u64 = digits(whole).parse().ok()?;
    let hundredths: u64 = match part.len() {
        0 => 0,
        1 => part.parse::<u64>().ok()? * 10,
        _ => part.parse().ok()?,
    };
    u32::try_from(units.checked_mul(100)?.checked_add(hundredths)?).ok()
}

/// Reads an answer of the market's search. Each result has the item's
/// market name, its cheapest listing written out as a price, the fees that
/// are part of that price, and the currency all of it is in. A price is only
/// gone by when its fees fit it: more than nothing, less than the price and
/// at least a twenty-fifth of it. That holds in every currency, so a price
/// read a hundred times too high or too low doesn't pass.
pub fn parse_search(text: &str) -> Result<SearchPage> {
    let v: Value = serde_json::from_str(text).map_err(|_| anyhow!("Steam answered with a page instead of search results"))?;
    let data = v.get("data").unwrap_or(&v);
    let results = data
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Steam's search answer isn't in the expected form"))?;
    let total = data.get("total_count").and_then(Value::as_u64).map_or(results.len(), |n| n as usize);
    let found = results
        .iter()
        .filter_map(|r| {
            let hash_name = r.get("strHash")?.as_str()?.to_string();
            let currency = r.get("eCurrency")?.as_u64().filter(|c| (1..=1000).contains(c))? as u32;
            let fee = |name: &str| r.get(name).and_then(Value::as_u64).unwrap_or(0);
            let fees = fee("unSteamFee") + fee("unPublisherFee");
            let lowest_sell = r
                .get("strMinSellSubtotal")
                .and_then(Value::as_str)
                .and_then(money_cents)
                .filter(|&price| fees > 0 && fees < price as u64 && fees * 25 >= price as u64);
            let sell_listings = r.get("cSellOrders").and_then(Value::as_u64).map_or(0, |n| n.min(u32::MAX as u64) as u32);
            Some(Found { hash_name, currency, lowest_sell, sell_listings })
        })
        .collect();
    Ok(SearchPage { total, results: results.len(), found })
}

/// One answer of the market's search: up to [`SEARCH_PAGE`] items of some
/// games, from `start` on, each with its cheapest listing. One request
/// where the order books of as many items take one each. "Too many
/// requests" comes back as a [`SteamBusy`](super::community::SteamBusy) error.
pub async fn search(ws: &WebSession, what: &SearchFor, currency: u32, start: usize) -> Result<SearchPage> {
    let text = ws.get_query_or_busy(&search_url(what, currency, start)).await.map_err(|e| plain_refusal(e, "market search"))?;
    parse_search(&text)
}

// -------------------------------------------------------- selling/listings --

pub async fn sell(ws: &WebSession, asset_id: &str, seller_cents: u32) -> Result<SellOutcome> {
    let resp = ws
        .client
        .post("https://steamcommunity.com/market/sellitem/")
        .header("Referer", format!("https://steamcommunity.com/profiles/{}/inventory/", ws.steam_id))
        .header("Origin", "https://steamcommunity.com")
        .form(&[
            ("sessionid", ws.session_id.as_str()),
            ("appid", "753"),
            ("contextid", "6"),
            ("assetid", asset_id),
            ("amount", "1"),
            ("price", &seller_cents.to_string()),
        ])
        .logged()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    let ok = v.get("success").and_then(Value::as_bool).unwrap_or(false);
    let needs_confirmation = v.get("requires_confirmation").and_then(Value::as_i64).unwrap_or(0) == 1
        || v.get("needs_mobile_confirmation").and_then(Value::as_bool).unwrap_or(false)
        || v.get("needs_email_confirmation").and_then(Value::as_bool).unwrap_or(false);
    let message = v
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| if ok { String::new() } else { format!("HTTP {status}") });
    Ok(SellOutcome { asset_id: asset_id.to_string(), ok, needs_confirmation, message })
}

pub fn parse_listings_html(html: &str) -> Vec<Listing> {
    let doc = Html::parse_fragment(html);
    let section_sel = sel!(".my_listing_section");
    let row_sel = sel!("div.market_listing_row");
    let header_sel = sel!(".my_market_header_active, .my_market_header");
    let name_sel = sel!(".market_listing_item_name");
    let game_sel = sel!(".market_listing_game_name");
    let price_sel = sel!(".market_listing_price");
    let id_re = re!(r"mylisting_(\d+)");
    let squash = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push_rows = |root: scraper::ElementRef, status: &str| {
        for row in root.select(&row_sel) {
            let Some(id) = row.value().attr("id").and_then(|i| id_re.captures(i)).map(|c| c[1].to_string()) else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            let text = |s: &Selector| {
                row.select(s)
                    .next()
                    .map(|e| squash(e.text().collect::<Vec<_>>().join(" ")))
                    .unwrap_or_default()
            };
            out.push(Listing {
                listing_id: id,
                name: text(&name_sel),
                game: text(&game_sel),
                price_text: text(&price_sel),
                status: status.to_string(),
            });
        }
    };
    let sections: Vec<_> = doc.select(&section_sel).collect();
    if sections.is_empty() {
        push_rows(doc.root_element(), "active");
    } else {
        for section in sections {
            let header = section
                .select(&header_sel)
                .next()
                .map(|h| h.text().collect::<String>().to_lowercase())
                .unwrap_or_default();
            let status = if header.contains("confirmation") { "pending" } else { "active" };
            push_rows(section, status);
        }
    }
    out
}

pub async fn my_listings(ws: &WebSession) -> Result<Vec<Listing>> {
    let mut all = Vec::new();
    let mut start = 0u32;
    loop {
        let url = format!("https://steamcommunity.com/market/mylistings/render/?query=&start={start}&count=100");
        let text = ws.get_text(&url).await?;
        let v: Value = serde_json::from_str(&text).context("listings JSON")?;
        if v.get("success").and_then(Value::as_bool) != Some(true) {
            bail!("Steam did not return your listings (web session expired?)");
        }
        let html = v.get("results_html").and_then(Value::as_str).unwrap_or("");
        let page = parse_listings_html(html);
        let total = v.get("total_count").and_then(Value::as_u64).unwrap_or(0) as u32;
        let got = page.len() as u32;
        all.extend(page);
        start += 100;
        if got == 0 || start >= total || start >= 2000 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    Ok(all)
}

pub async fn remove_listing(ws: &WebSession, listing_id: &str) -> Result<()> {
    let resp = ws
        .client
        .post(format!("https://steamcommunity.com/market/removelisting/{listing_id}"))
        .header("Referer", "https://steamcommunity.com/market/")
        .header("Origin", "https://steamcommunity.com")
        .form(&[("sessionid", ws.session_id.as_str())])
        .logged()
        .await?;
    if !resp.status().is_success() {
        bail!("HTTP {} removing listing {listing_id}", resp.status());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_math_roundtrips() {
        let usd = Wallet::default();
        assert_eq!(usd.buyer_pays(1), 3);
        assert_eq!(usd.buyer_pays(100), 115);
        assert_eq!(usd.buyer_pays(20), 23);
        assert_eq!(usd.lowest_price(), 3);
        assert_eq!(usd.seller_receives(2), 0);
        // a wallet with bigger minimums, a base fee and coarser prices
        let other = Wallet { steam_minimum: 30, fee_base: 5, increment: 10, market_minimum: 50, ..usd };
        assert_eq!(other.buyer_pays(100), 100 + 35 + 10);
        assert_eq!(other.buyer_pays(1000), 1000 + 55 + 100);
        assert_eq!(other.lowest_price(), 50 + 35 + 5);
        assert_eq!(other.seller_receives(other.lowest_price() - 1), 0);
        // a wallet whose games take no share
        let no_share = Wallet { publisher_percent: 0.0, ..usd };
        assert_eq!(no_share.buyer_pays(100), 105);
        for wallet in [usd, other, no_share] {
            for b in wallet.lowest_price()..3000u32 {
                let s = wallet.seller_receives(b);
                assert!(s > 0 && wallet.buyer_pays(s) <= b, "b={b} s={s} {wallet:?}");
                assert!(wallet.buyer_pays(s + 1) > b, "not maximal at b={b} {wallet:?}");
            }
        }
    }

    #[test]
    fn wallet_block_of_the_market_page() {
        let page = r#"<script>var g_rgWalletInfo = {"wallet_currency":31,"wallet_country":"SA","wallet_fee":"1",
            "wallet_fee_minimum":"4","wallet_fee_percent":"0.05","wallet_publisher_fee_percent_default":"0.10",
            "wallet_fee_base":"0","wallet_currency_increment":"1","wallet_market_minimum":"4","wallet_balance":"1250","success":1};</script>"#;
        let w = Wallet::parse(page).unwrap();
        assert_eq!(w, Wallet { currency: 31, steam_minimum: 4, market_minimum: 4, ..Wallet::default() });
        assert_eq!(w.buyer_pays(4), 4 + 4 + 1);
        assert_eq!(w.lowest_price(), 9);
        // signed out, or a page without the block
        assert_eq!(Wallet::parse("<html></html>"), None);
        // nonsense is not believed
        let odd = Wallet::parse(r#"{"wallet_currency":3,"wallet_fee_percent":"7","wallet_fee_minimum":"0"}"#);
        assert_eq!(odd, Some(Wallet { currency: 3, ..Wallet::default() }));
    }

    #[test]
    fn pricing_strategies() {
        let w = Wallet::default();
        assert_eq!(suggest_price(&w, Some(20), Some(15), "undercutLowest", 1, 3, 0), Some(19));
        assert_eq!(suggest_price(&w, Some(20), Some(19), "undercutLowest", 1, 3, 0), Some(20));
        assert_eq!(suggest_price(&w, Some(20), Some(15), "matchLowest", 1, 3, 0), Some(20));
        assert_eq!(suggest_price(&w, Some(20), Some(15), "aboveHighestBuy", 1, 3, 0), Some(16));
        assert_eq!(suggest_price(&w, Some(4), None, "undercutLowest", 5, 10, 0), Some(10));
        assert_eq!(suggest_price(&w, Some(500), None, "undercutLowest", 1, 3, 100), Some(100));
        assert_eq!(suggest_price(&w, None, None, "undercutLowest", 1, 3, 0), None);
        assert_eq!(suggest_price(&w, None, Some(7), "undercutLowest", 1, 3, 0), Some(8));
        assert_eq!(suggest_price(&w, Some(20), Some(15), "undercutLowest", 0, 0, 0), Some(20));
        assert_eq!(suggest_price(&w, Some(1), None, "undercutLowest", 1, 0, 0), Some(3));
        // prices move in steps of 10 and can't go under the wallet's lowest price
        let coarse = Wallet { steam_minimum: 30, increment: 10, market_minimum: 50, ..w };
        assert_eq!(suggest_price(&coarse, Some(500), Some(300), "undercutLowest", 1, 0, 0), Some(490));
        assert_eq!(suggest_price(&coarse, Some(500), Some(300), "aboveHighestBuy", 1, 0, 0), Some(310));
        assert_eq!(suggest_price(&coarse, Some(60), None, "undercutLowest", 1, 0, 0), Some(coarse.lowest_price()));
    }

    #[test]
    fn inventory_grouping() {
        let v: Value = serde_json::from_str(r#"{
          "assets":[{"assetid":"1","classid":"10","instanceid":"0"},{"assetid":"2","classid":"10","instanceid":"0"},{"assetid":"3","classid":"11","instanceid":"0"}],
          "descriptions":[
            {"classid":"10","instanceid":"0","market_hash_name":"730-Card A","name":"Card A","type":"CS2 Trading Card","marketable":1,"icon_url":"abc",
             "tags":[{"category":"item_class","internal_name":"item_class_2"},{"category":"cardborder","internal_name":"cardborder_0"},{"category":"Game","localized_tag_name":"Counter-Strike 2"}]},
            {"classid":"11","instanceid":"0","market_hash_name":"753-Gems","name":"Gems","marketable":0,"tags":[{"category":"item_class","internal_name":"item_class_7"}]}],
          "more_items":1,"last_assetid":"3"}"#).unwrap();
        let mut groups = BTreeMap::new();
        let (more, last) = parse_inventory_page(&v, &mut groups);
        assert!(more);
        assert_eq!(last.as_deref(), Some("3"));
        let a = &groups["730-Card A"];
        assert_eq!(a.count, 2);
        assert_eq!(a.asset_ids, vec!["1", "2"]);
        assert_eq!(a.class, "card");
        assert_eq!(a.game, "Counter-Strike 2");
        assert!(a.marketable);
        assert!(!groups["753-Gems"].marketable);
        assert_eq!(groups["753-Gems"].class, "gems");
    }

    #[test]
    fn listings_sections() {
        let html = r#"
        <div class="my_listing_section"><h3 class="my_market_header"><span class="my_market_header_active">My sell listings</span></h3>
          <div class="market_listing_row market_recent_listing_row" id="mylisting_111">
            <span class="market_listing_price">$0.10 ($0.08)</span>
            <span class="market_listing_item_name">Card A</span><span class="market_listing_game_name">Game X Trading Card</span>
          </div></div>
        <div class="my_listing_section"><h3 class="my_market_header"><span class="my_market_header_active">My listings awaiting confirmation</span></h3>
          <div class="market_listing_row" id="mylisting_222"><span class="market_listing_item_name">Card B</span></div></div>"#;
        let l = parse_listings_html(html);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].listing_id, "111");
        assert_eq!(l[0].status, "active");
        assert_eq!(l[0].price_text, "$0.10 ($0.08)");
        assert_eq!(l[1].status, "pending");
        assert_eq!(l[1].name, "Card B");
    }

    #[test]
    fn order_book_by_name_as_steam_answers_it() {
        // Steam's answer for a trading card (riyals), November 2026.
        let book = parse_order_book(include_str!("fixtures/orderbook_card.json")).unwrap();
        assert_eq!(
            book,
            Some(Book { currency: 31, lowest_sell: Some(21), highest_buy: Some(14), sell_listings: 1141 })
        );
        // Steam has no market for the item.
        assert_eq!(parse_order_book(r#"{"data":{"success":false}}"#).unwrap(), None);
        // Nobody selling, nobody buying.
        let empty = r#"{"data":{"success":true,"data":{"amtMaxBuyOrder":0,"eCurrency":1,"cBuyOrders":0,"cSellOrders":0,"rgCompactBuyOrders":[],"rgCompactSellOrders":[]}}}"#;
        assert_eq!(
            parse_order_book(empty).unwrap(),
            Some(Book { currency: 1, lowest_sell: None, highest_buy: None, sell_listings: 0 })
        );
        // Unwrapped, should Steam drop the outer layer.
        let bare = r#"{"success":true,"data":{"amtMinSellOrder":7,"eCurrency":3,"cSellOrders":2}}"#;
        assert_eq!(parse_order_book(bare).unwrap().unwrap().lowest_sell, Some(7));
        // What can't be read is an error, not "no price".
        assert!(parse_order_book("<html>Sorry!</html>").is_err());
        assert!(parse_order_book(r#"{"data":{"success":true,"data":{"amtMinSellOrder":7}}}"#).is_err());
        assert!(parse_order_book(r#"{"data":[]}"#).is_err());
    }

    #[test]
    fn wallet_of_the_market_steam_opened_in_november_2026() {
        // The page's data says riyals, in hundredths, and nothing of fees.
        let page = include_str!("fixtures/market_page_2026.html");
        assert_eq!(Wallet::parse(page), None);
        let w = Wallet::read(page).unwrap();
        assert_eq!(w, Wallet::standard(31, false));
        assert_eq!((w.currency, w.increment, w.steam_percent, w.publisher_percent), (31, 1, 0.05, 0.10));
        assert_eq!(w.buyer_pays(19), 21);

        // A currency in whole units only: every price is a whole number of them.
        let whole = page.replace(r#"\"bWholeUnitsOnly\":false,\"eCurrency\":31"#, r#"\"bWholeUnitsOnly\":true,\"eCurrency\":16"#);
        assert_ne!(whole, page);
        let w = Wallet::read(&whole).unwrap();
        assert_eq!((w.currency, w.increment, w.market_minimum), (16, 100, 100));

        // Should the page carry the old wallet numbers, signed in, those count.
        let old = page.replace(
            r#"\"marketEligibility\":"#,
            r#"\"walletInfo\":{\"wallet_currency\":3,\"wallet_fee_percent\":\"0.05\",\"wallet_fee_minimum\":1,\"wallet_publisher_fee_percent_default\":\"0.10\",\"wallet_fee_base\":0},\"marketEligibility\":"#,
        );
        assert_ne!(old, page);
        assert_eq!(Wallet::read(&old).map(|w| w.currency), Some(3));

        // No page data, no wallet.
        assert_eq!(Wallet::read("<html><body>Steam is down</body></html>"), None);
    }

    #[test]
    fn market_search_as_steam_answers_it() {
        // Steam's answer for Team Fortress 2's cards (US dollars: nobody was
        // signed in), November 2026, each description cut down to a few fields.
        let page = parse_search(include_str!("fixtures/market_search_cards.json")).unwrap();
        assert_eq!((page.total, page.results, page.found.len()), (9, 9, 9));
        assert_eq!(
            page.found[0],
            Found { hash_name: "440-DEMOMAN (Trading Card)".into(), currency: 1, lowest_sell: Some(4), sell_listings: 5495 }
        );
        assert_eq!(page.found[4].hash_name, "440-PYRO (Trading Card)");
        assert_eq!((page.found[4].lowest_sell, page.found[4].sell_listings), (Some(5), 4896));

        let one = |price: &str, steam: u32, publisher: u32| {
            let text = format!(
                r#"{{"data":{{"start":0,"total_count":1,"results":[{{"strHash":"1-A","eCurrency":31,"cSellOrders":3,"strMinSellSubtotal":"{price}","unSteamFee":{steam},"unPublisherFee":{publisher}}}]}}}}"#
            );
            parse_search(&text).unwrap().found[0].clone()
        };
        // Riyals: 0.20 with 0.04 + 0.04 of fees in it.
        assert_eq!(one("0.20 SR", 4, 4), Found { hash_name: "1-A".into(), currency: 31, lowest_sell: Some(20), sell_listings: 3 });
        // Steam lists some items at no price at all: that is no price.
        assert_eq!(one("$0.00", 0, 0).lowest_sell, None);
        // A price whose fees don't fit it was read wrongly, and isn't gone by:
        // a hundred times too high, too low, or with no fees to go by.
        assert_eq!(one("5.000 X", 1, 1).lowest_sell, None);
        assert_eq!(one("0,05€", 100, 100).lowest_sell, None);
        assert_eq!(one("$0.05", 0, 0).lowest_sell, None);
        // The lowest price there is, and a high one.
        assert_eq!(one("$0.03", 1, 1).lowest_sell, Some(3));
        assert_eq!(one("$100.00", 434, 869).lowest_sell, Some(10_000));

        // Nothing found is an answer too; what can't be read is an error.
        let none = parse_search(r#"{"data":{"start":0,"total_count":0,"grouping":1,"facets":[],"results":[]}}"#).unwrap();
        assert_eq!(none, SearchPage::default());
        assert!(parse_search("<html>Sorry!</html>").is_err());
        assert!(parse_search(r#"{"data":{"success":false}}"#).is_err());
        // A result without its name or currency is left out, and still counted.
        let odd = parse_search(r#"{"data":{"total_count":2,"results":[{"strHash":"1-A"},{"eCurrency":1}]}}"#).unwrap();
        assert_eq!((odd.total, odd.results, odd.found.len()), (2, 2, 0));
    }

    #[test]
    fn prices_as_steam_writes_them_out() {
        for (text, cents) in [
            ("$0.05", 5),
            ("$1,234.56", 123_456),
            ("0,05€", 5),
            ("5,--€", 500),
            ("0.19 SR", 19),
            ("R$ 0,35", 35),
            ("S/.0.19", 19),
            ("1.234,50 TL", 123_450),
            ("1 234,56 zł", 123_456),
            ("12,5 pуб.", 1250),
            ("¥ 6", 600),
            ("₩ 1,250", 125_000),
            ("CLP$ 1.234", 123_400),
            ("Rp 2 500", 250_000),
            ("₹ 1,23,456", 12_345_600),
            ("$0.00", 0),
        ] {
            assert_eq!(money_cents(text), Some(cents), "{text}");
        }
        assert_eq!(money_cents(""), None);
        assert_eq!(money_cents("free"), None);
        // Too much to be a price.
        assert_eq!(money_cents("99999999999999999999"), None);
    }

    #[test]
    fn a_search_asks_for_one_kind_of_item_of_some_games() {
        let asked = |kind, start| {
            let url = search_url(&SearchFor { kind, games: vec![440, 730] }, 31, start);
            let (at, query) = url.split_once("&qp=").unwrap();
            assert_eq!(at, "https://steamcommunity.com/market/actions?q=Search");
            serde_json::from_str::<Value>(&urlencoding::decode(query).unwrap()).unwrap()
        };
        let cards = asked(SearchKind::Cards, 200);
        assert_eq!(
            cards,
            json!([{
                "appid": 753,
                "filters": { "Game": ["app_440", "app_730"], "item_class": ["item_class_2"], "cardborder": ["cardborder_0"] },
                "accessoryFilters": {},
                "price": { "eCurrency": 31 },
                "start": 200,
                "count": 100,
                "sort": 1,
                "direction": 1
            }])
        );
        assert_eq!(asked(SearchKind::Foils, 0)[0]["filters"]["cardborder"], json!(["cardborder_1"]));
        let others = asked(SearchKind::Others, 0);
        assert_eq!(others[0]["filters"]["item_class"], json!(["item_class_3", "item_class_4", "item_class_5"]));
        assert_eq!(others[0]["filters"].get("cardborder"), None);

        assert_eq!(SearchKind::of_class("card"), Some(SearchKind::Cards));
        assert_eq!(SearchKind::of_class("foil"), Some(SearchKind::Foils));
        for class in ["background", "emoticon", "booster"] {
            assert_eq!(SearchKind::of_class(class), Some(SearchKind::Others));
        }
        assert_eq!(SearchKind::of_class("gems"), None);
        assert_eq!(SearchKind::of_class("other"), None);
    }

    #[test]
    fn currencies_by_their_letters() {
        assert_eq!(currency_code(1), "USD");
        assert_eq!(currency_code(3), "EUR");
        assert_eq!(currency_code(31), "SAR");
        assert_eq!(currency_code(41), "UYU");
        // Steam has none with these numbers.
        for unknown in [0, 33, 36, 42, 999] {
            assert_eq!(currency_code(unknown), format!("currency {unknown}"));
        }
    }
}
