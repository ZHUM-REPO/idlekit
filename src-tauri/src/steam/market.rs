//! Steam Community Market: inventory (app 753 / context 6 — cards, emoticons,
//! backgrounds, boosters), live order-book pricing, listing and delisting.
use super::community::WebSession;
use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use scraper::{Html, Selector};
use serde::Serialize;
use serde_json::Value;
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
    /// Prices are what the buyer pays, in the wallet currency's minor units.
    pub lowest_sell: Option<u32>,
    pub highest_buy: Option<u32>,
    pub sell_listings: u32,
    /// Suggested buyer price after strategy and min/max limits.
    pub suggested: Option<u32>,
    /// What you receive after Steam's fees for `suggested`.
    pub you_receive: Option<u32>,
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
/// publishes these numbers on the market page (`g_rgWalletInfo`) and its own
/// sell dialog computes from them; the defaults are a US-dollar wallet's.
/// Amounts are in the currency's minor units.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
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
        let field = |name: &str| -> Option<f64> {
            Regex::new(&format!(r#""{name}"\s*:\s*"?([0-9]+(?:\.[0-9]+)?)"?"#))
                .ok()?
                .captures(html)
                .and_then(|c| c[1].parse::<f64>().ok())
        };
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

pub async fn wallet(ws: &WebSession) -> Result<Wallet> {
    let html = ws.get_text("https://steamcommunity.com/market/").await?;
    Wallet::parse(&html).ok_or_else(|| anyhow!("Steam's market page showed no wallet"))
}

pub async fn item_name_id(ws: &WebSession, hash_name: &str) -> Result<u64> {
    let url = format!(
        "https://steamcommunity.com/market/listings/753/{}",
        urlencoding::encode(hash_name)
    );
    let html = ws.get_text(&url).await?;
    Regex::new(r"Market_LoadOrderSpread\(\s*(\d+)\s*\)")?
        .captures(&html)
        .and_then(|c| c[1].parse().ok())
        .ok_or_else(|| anyhow!("no order book for {hash_name}"))
}

fn cents(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64().map(|x| x as u32),
        _ => None,
    }
    .filter(|&c| c > 0)
}

pub async fn order_book(ws: &WebSession, item_nameid: u64, currency: u32) -> Result<(Option<u32>, Option<u32>, u32)> {
    let url = format!(
        "https://steamcommunity.com/market/itemordershistogram?country=US&language=english&currency={currency}&item_nameid={item_nameid}&two_factor=0&norender=1"
    );
    let text = ws.get_text(&url).await?;
    let v: Value = serde_json::from_str(&text).context("order book JSON")?;
    if v.get("success").and_then(Value::as_i64) != Some(1) {
        bail!("Steam refused the price lookup");
    }
    let sell_count = v
        .get("sell_order_count")
        .or_else(|| v.get("sell_order_summary"))
        .and_then(|c| match c {
            Value::String(s) => s.replace(',', "").parse().ok(),
            Value::Number(n) => n.as_u64().map(|x| x as u32),
            _ => None,
        })
        .unwrap_or(0);
    Ok((cents(v.get("lowest_sell_order")), cents(v.get("highest_buy_order")), sell_count))
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
        .send()
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
    let section_sel = Selector::parse(".my_listing_section").unwrap();
    let row_sel = Selector::parse("div.market_listing_row").unwrap();
    let header_sel = Selector::parse(".my_market_header_active, .my_market_header").unwrap();
    let name_sel = Selector::parse(".market_listing_item_name").unwrap();
    let game_sel = Selector::parse(".market_listing_game_name").unwrap();
    let price_sel = Selector::parse(".market_listing_price").unwrap();
    let id_re = Regex::new(r"mylisting_(\d+)").unwrap();
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
        .send()
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
}
