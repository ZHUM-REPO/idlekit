//! The window's commands for the Counter-Strike 2 market and for trade
//! offers. Each writes down what it did in words; what moves money or items
//! (buying, selling, sending or accepting an offer) is sent to Steam once,
//! never again by itself, and says how it ended either way.
use crate::commands::{anyhow_err, progress, wallet_for_prices, CmdResult};
use crate::logbus;
use crate::state::App;
use crate::steam::community;
use crate::steam::cs2::{self, About, Bought, Buyer, Cs2Item, Depth, Facet, History, Hits, ItemPage, Mine, OrderStatus, Query, Suggestion, ToBuy};
use crate::steam::market::{self, currency_code, SellOutcome, Wallet};
use crate::steam::trade::{self, AssetRef, Offers, TradeOutcome};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::State;

/// The most of one item a single buy order may ask for from here: a slip of
/// the hand shouldn't order a thousand.
pub const MOST_PER_ORDER: u32 = 100;

/// An error in the words the window shows.
fn told(e: anyhow::Error) -> String {
    match community::steam_busy(&e) {
        Some(429) => "Steam says there were too many requests. Wait a minute, then try again".into(),
        Some(code) => format!("Steam had trouble answering (HTTP {code}). Try again in a moment"),
        None => anyhow_err(e),
    }
}

/// An amount in words: "SAR 86.07".
fn amount(minor: u64, currency: u32) -> String {
    format!("{} {}.{:02}", currency_code(currency), minor / 100, minor % 100)
}

/// A long number (a listing's, an offer's) by its last four digits: enough
/// to tell two apart in the log.
fn short(id: &str) -> String {
    let n = id.chars().count();
    if n <= 4 { id.to_string() } else { format!("…{}", id.chars().skip(n - 4).collect::<String>()) }
}

fn player(steam_id: &str) -> String {
    logbus::label(steam_id).replacen("account", "player", 1)
}

// ---------------------------------------------------------------- browse ---

#[tauri::command]
pub async fn cs2_facets(app: State<'_, App>, steam_id: String) -> CmdResult<Vec<Facet>> {
    let started = Instant::now();
    let facets = app.with_web(&steam_id, |ws| async move { cs2::facets(&ws).await }).await.map_err(told)?;
    logbus::debug(
        "cs2",
        format!("Read the market's filters for CS2: {} with {} tag(s) in {} ms", facets.len(), facets.iter().map(|f| f.tags.len()).sum::<usize>(), started.elapsed().as_millis()),
    );
    Ok(facets)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchAnswer {
    #[serde(flatten)]
    pub hits: Hits,
    /// The wallet's currency, which the prices were asked in. Steam may
    /// answer in another: each result says which.
    pub wallet_currency: u32,
}

/// How a search is named in the log: its words, how much narrows it, its order.
fn search_words(q: &Query) -> String {
    let order = match (q.sort, q.direction) {
        (1, 1) => "by name",
        (1, _) => "by name, last first",
        (2, 1) => "fewest listed first",
        (2, _) => "most listed first",
        (3, 1) => "cheapest first",
        (3, _) => "dearest first",
        (4, _) => "by popularity",
        _ => "trending first",
    };
    let picked: usize = q.filters.values().map(Vec::len).sum();
    let mut parts = Vec::new();
    if !q.text.trim().is_empty() {
        parts.push(format!("\"{}\"{}", q.text.trim(), if q.descriptions { " in descriptions too" } else { "" }));
    }
    if picked > 0 {
        parts.push(format!("{picked} filter tag(s)"));
    }
    if q.price_min.is_some_and(|p| p > 0) || q.price_max.is_some_and(|p| p > 0) {
        parts.push("a price range".into());
    }
    parts.push(order.into());
    parts.join(", ")
}

#[tauri::command]
pub async fn cs2_search(app: State<'_, App>, steam_id: String, query: Query) -> CmdResult<SearchAnswer> {
    let started = Instant::now();
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let wallet = wallet_for_prices(&app, &ws, &steam_id).await;
    let hits = match cs2::search(&ws, &query, wallet.currency).await {
        Ok(hits) => hits,
        Err(e) => {
            app.web_failed(&steam_id, &ws, &e);
            let why = told(e);
            logbus::warn("cs2", format!("CS2 search ({}): {why}", search_words(&query)));
            return Err(why);
        }
    };
    let other = hits.results.iter().map(|h| h.currency).find(|&c| c != 0 && c != wallet.currency);
    logbus::debug(
        "cs2",
        format!(
            "CS2 search ({}): {} item(s) in all, {} from {} on, in {} ms{}{}",
            search_words(&query),
            hits.total,
            hits.results.len(),
            hits.start,
            started.elapsed().as_millis(),
            match other {
                Some(c) => format!("; prices came in {}, not the wallet's {}", currency_code(c), currency_code(wallet.currency)),
                None => String::new(),
            },
            if query.descriptions && !query.text.trim().is_empty() && !hits.in_descriptions { "; Steam gave the description search no answer, so names were searched" } else { "" },
        ),
    );
    Ok(SearchAnswer { hits, wallet_currency: wallet.currency })
}

#[tauri::command]
pub async fn cs2_suggest(app: State<'_, App>, steam_id: String, typed: String) -> CmdResult<Vec<Suggestion>> {
    if typed.trim().chars().count() < 3 {
        return Ok(Vec::new());
    }
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    // The box works without them: no suggestions is no error.
    Ok(cs2::suggestions(&ws, &typed).await.unwrap_or_else(|e| {
        logbus::debug("cs2", format!("No suggestions for the search box: {e:#}"));
        Vec::new()
    }))
}

#[tauri::command]
pub async fn cs2_item(app: State<'_, App>, steam_id: String, id: String, filters: BTreeMap<String, Vec<String>>, start: usize) -> CmdResult<ItemPage> {
    let started = Instant::now();
    let asked = id.clone();
    let page = app
        .with_web(&steam_id, |ws| {
            let (id, filters) = (id.clone(), filters.clone());
            async move { cs2::item(&ws, &id, &filters, start).await }
        })
        .await
        .map_err(|e| {
            let why = told(e);
            logbus::warn("cs2", format!("Listings of {asked}: {why}"));
            why
        })?;
    logbus::debug(
        "cs2",
        format!("Listings of {asked}: {} of {} from {} on, {} kind tag(s), in {} ms", page.listings.len(), page.total, page.start, page.facets.len(), started.elapsed().as_millis()),
    );
    Ok(page)
}

#[tauri::command]
pub async fn cs2_about(app: State<'_, App>, steam_id: String, hash_name: String) -> CmdResult<Option<About>> {
    let name = hash_name.clone();
    let about = app.with_web(&steam_id, |ws| { let name = name.clone(); async move { cs2::about(&ws, &name).await } }).await.map_err(told)?;
    logbus::debug("cs2", format!("{hash_name}: {}", if about.is_some() { "read its description" } else { "Steam has no description for it" }));
    Ok(about)
}

#[tauri::command]
pub async fn cs2_book(app: State<'_, App>, steam_id: String, hash_name: String) -> CmdResult<Option<Depth>> {
    let name = hash_name.clone();
    let depth = app.with_web(&steam_id, |ws| { let name = name.clone(); async move { cs2::depth(&ws, &name).await } }).await.map_err(|e| {
        let why = told(e);
        logbus::warn("cs2", format!("Order book of {hash_name}: {why}"));
        why
    })?;
    match &depth {
        Some(d) => logbus::debug(
            "cs2",
            format!(
                "Order book of {hash_name}: {} for sale from {}, {} wanted up to {}",
                d.sell_count,
                d.lowest_sell.map_or("no price".into(), |p| amount(p as u64, d.currency)),
                d.buy_count,
                d.highest_buy.map_or("no price".into(), |p| amount(p as u64, d.currency)),
            ),
        ),
        None => logbus::debug("cs2", format!("Order book of {hash_name}: Steam has no market for it")),
    }
    Ok(depth)
}

#[tauri::command]
pub async fn cs2_history(app: State<'_, App>, steam_id: String, hash_name: String) -> CmdResult<Option<History>> {
    let name = hash_name.clone();
    let history = app.with_web(&steam_id, |ws| { let name = name.clone(); async move { cs2::history(&ws, &name).await } }).await.map_err(told)?;
    logbus::debug("cs2", format!("Price history of {hash_name}: {} point(s)", history.as_ref().map_or(0, |h| h.points.len())));
    Ok(history)
}

// ---------------------------------------------------------------- buying ---

/// The account as a buyer, with the fee numbers the window computes with.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuyerView {
    #[serde(flatten)]
    pub buyer: Buyer,
    pub wallet: Wallet,
}

#[tauri::command]
pub async fn cs2_buyer(app: State<'_, App>, steam_id: String) -> CmdResult<BuyerView> {
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let buyer = match cs2::buyer(&ws).await {
        Ok(b) => b,
        Err(e) => {
            app.web_failed(&steam_id, &ws, &e);
            let why = told(e);
            logbus::warn("cs2", format!("The wallet couldn't be read: {why}"));
            return Err(why);
        }
    };
    let wallet = wallet_for_prices(&app, &ws, &steam_id).await;
    // Steam's standard fees in the currency the wallet is in, whatever the market page said.
    let wallet = if wallet.currency == buyer.currency { wallet } else { Wallet { currency: buyer.currency, ..wallet } };
    logbus::debug(
        "cs2",
        format!(
            "Wallet of {}: {}{}{}",
            logbus::label(&steam_id),
            buyer.balance.map_or_else(|| format!("in {}", currency_code(buyer.currency)), |b| amount(b, buyer.currency)),
            match buyer.confirmation {
                2 => ", purchases confirmed in the mobile app",
                1 => ", purchases confirmed by e-mail",
                _ => "",
            },
            if buyer.needs_billing { ", Steam asks for a billing address with purchases" } else { "" },
        ),
    );
    Ok(BuyerView { buyer, wallet })
}

/// What stands in the way of a purchase before Steam is asked: a price in
/// another currency than the wallet's, a wallet too small, or an address
/// Steam wants and doesn't have.
pub fn purchase_objection(buyer: &Buyer, currency: u32, total: u64) -> Option<String> {
    if currency != buyer.currency {
        return Some(format!(
            "This price is in {} and the wallet is in {}. Open the item again so Steam shows it in the wallet's currency",
            currency_code(currency),
            currency_code(buyer.currency)
        ));
    }
    if let Some(balance) = buyer.balance {
        if total > balance {
            return Some(format!("The wallet has {} and this costs {}", amount(balance, currency), amount(total, currency)));
        }
    }
    if buyer.needs_billing && !buyer.has_billing {
        return Some("Steam asks for a billing address with purchases from your country and has none saved. Buy once on Steam's own market page, which saves it".into());
    }
    None
}

#[tauri::command]
pub async fn cs2_buy_listing(app: State<'_, App>, steam_id: String, listing: ToBuy, confirmation: Option<String>) -> CmdResult<Bought> {
    let what = format!("{} (listing {})", listing.hash_name, short(&listing.listing_id));
    if listing.price == 0 || listing.fee == 0 {
        return Err("This listing has no price to buy it at".into());
    }
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let fail = |e: anyhow::Error, doing: &str| {
        app.web_failed(&steam_id, &ws, &e);
        let why = told(e);
        logbus::warn("cs2", format!("Buying {what}: {doing}: {why}"));
        why
    };
    let buyer = cs2::buyer(&ws).await.map_err(|e| fail(e, "the wallet couldn't be read"))?;
    let total = listing.price as u64 + listing.fee as u64 + buyer.tax_on(listing.fee) as u64;
    if let Some(objection) = purchase_objection(&buyer, listing.currency, total) {
        logbus::warn("cs2", format!("Buying {what}: not sent to Steam. {objection}"));
        return Err(objection);
    }
    let price = format!(
        "{} ({} to the seller, {} in fees)",
        amount(total, buyer.currency),
        amount(listing.price as u64, buyer.currency),
        amount(total - listing.price as u64, buyer.currency)
    );
    // The first try is worth a line of its own; asking again whether it was confirmed isn't.
    match &confirmation {
        None => logbus::info("cs2", format!("Buying {what} for {price}")),
        Some(_) => logbus::debug("cs2", format!("Buying {what}: asking Steam whether it has been confirmed")),
    }
    let bought = cs2::buy_listing(&ws, &listing, &buyer, confirmation.as_deref()).await.map_err(|e| fail(e, "Steam wasn't reached, or didn't answer; check the inventory before trying again"))?;
    if bought.ok {
        logbus::info(
            "cs2",
            format!("Bought {what} for {price}{}", bought.balance.map(|b| format!("; the wallet now has {}", amount(b, buyer.currency))).unwrap_or_default()),
        );
    } else if bought.needs_confirmation {
        let how = if buyer.confirmation == 1 { "by e-mail" } else { "in the Steam mobile app" };
        match &confirmation {
            None => logbus::info("cs2", format!("Buying {what}: Steam waits for it to be confirmed {how}")),
            Some(_) => logbus::debug("cs2", format!("Buying {what}: not confirmed yet")),
        }
    } else {
        logbus::warn("cs2", format!("Buying {what}: Steam refused. {}", bought.message));
    }
    Ok(bought)
}

#[tauri::command]
pub async fn cs2_buy_order(
    app: State<'_, App>,
    steam_id: String,
    hash_name: String,
    price_each: u32,
    quantity: u32,
    confirmation: Option<String>,
) -> CmdResult<Bought> {
    if quantity == 0 || quantity > MOST_PER_ORDER {
        return Err(format!("An order from here is for 1 to {MOST_PER_ORDER} of an item"));
    }
    if hash_name.trim().is_empty() {
        return Err("Which item the order is for is missing".into());
    }
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let what = format!("{quantity} × {hash_name}");
    let fail = |e: anyhow::Error, doing: &str| {
        app.web_failed(&steam_id, &ws, &e);
        let why = told(e);
        logbus::warn("cs2", format!("Buy order for {what}: {doing}: {why}"));
        why
    };
    let buyer = cs2::buyer(&ws).await.map_err(|e| fail(e, "the wallet couldn't be read"))?;
    let wallet = Wallet { currency: buyer.currency, ..wallet_for_prices(&app, &ws, &steam_id).await };
    if price_each < wallet.lowest_price() {
        return Err(format!("The lowest price an item can be bought at is {}", amount(wallet.lowest_price() as u64, buyer.currency)));
    }
    let total = price_each as u64 * quantity as u64;
    // An order may stand for more than the wallet holds (Steam lets all of
    // an account's orders add up to ten times its wallet), but never without a wallet.
    let allowed = buyer.balance.map(|b| b.saturating_mul(10));
    if let Some(objection) = purchase_objection(&Buyer { balance: allowed, ..buyer.clone() }, buyer.currency, total) {
        let objection = if allowed.is_some_and(|a| total > a) {
            format!("Steam lets buy orders add up to ten times the wallet ({}); this one is for {}", amount(allowed.unwrap_or(0), buyer.currency), amount(total, buyer.currency))
        } else {
            objection
        };
        logbus::warn("cs2", format!("Buy order for {what}: not sent to Steam. {objection}"));
        return Err(objection);
    }
    let price = format!("{} each, {} in all", amount(price_each as u64, buyer.currency), amount(total, buyer.currency));
    match &confirmation {
        None => logbus::info("cs2", format!("Placing a buy order for {what} at {price}")),
        Some(_) => logbus::debug("cs2", format!("Buy order for {what}: asking Steam whether it has been confirmed")),
    }
    let placed = cs2::create_buy_order(&ws, &hash_name, price_each, quantity, &buyer, &wallet, confirmation.as_deref())
        .await
        .map_err(|e| fail(e, "Steam wasn't reached, or didn't answer; look at My orders before trying again"))?;
    if placed.ok {
        logbus::info("cs2", format!("Buy order placed for {what} at {price}{}", placed.order_id.as_deref().map(|id| format!(" (order {})", short(id))).unwrap_or_default()));
    } else if placed.needs_confirmation {
        let how = if buyer.confirmation == 1 { "by e-mail" } else { "in the Steam mobile app" };
        match &confirmation {
            None => logbus::info("cs2", format!("Buy order for {what}: Steam waits for it to be confirmed {how}")),
            Some(_) => logbus::debug("cs2", format!("Buy order for {what}: not confirmed yet")),
        }
    } else {
        logbus::warn("cs2", format!("Buy order for {what}: Steam refused. {}", placed.message));
    }
    Ok(placed)
}

#[tauri::command]
pub async fn cs2_order_status(app: State<'_, App>, steam_id: String, order_id: String) -> CmdResult<OrderStatus> {
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let status = cs2::buy_order_status(&ws, &order_id).await.map_err(told)?;
    logbus::debug(
        "cs2",
        format!(
            "Buy order {}: {} of {} bought{}",
            short(&order_id),
            status.bought,
            status.quantity.max(status.bought),
            if status.active { ", still open" } else { ", closed" }
        ),
    );
    Ok(status)
}

#[tauri::command]
pub async fn cs2_cancel_order(app: State<'_, App>, steam_id: String, order_id: String) -> CmdResult<()> {
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    match cs2::cancel_buy_order(&ws, &order_id).await {
        Ok(()) => {
            logbus::info("cs2", format!("Cancelled buy order {}", short(&order_id)));
            Ok(())
        }
        Err(e) => {
            app.web_failed(&steam_id, &ws, &e);
            let why = told(e);
            logbus::warn("cs2", format!("Buy order {}: {why}", short(&order_id)));
            Err(why)
        }
    }
}

#[tauri::command]
pub async fn cs2_mine(app: State<'_, App>, steam_id: String) -> CmdResult<Mine> {
    let started = Instant::now();
    let mine = app.with_web(&steam_id, |ws| async move { cs2::mine(&ws).await }).await.map_err(told)?;
    logbus::debug(
        "cs2",
        format!("CS2 listings and buy orders of {}: {} listing(s), {} order(s) in {} ms", logbus::label(&steam_id), mine.listings.len(), mine.orders.len(), started.elapsed().as_millis()),
    );
    Ok(mine)
}

// --------------------------------------------------------------- selling ---

#[tauri::command]
pub async fn cs2_inventory(app: State<'_, App>, steam_id: String) -> CmdResult<Vec<Cs2Item>> {
    let started = Instant::now();
    let items = app.with_web(&steam_id, |ws| async move { cs2::inventory(&ws).await }).await.map_err(told)?;
    logbus::debug(
        "cs2",
        format!(
            "CS2 inventory of {}: {} item(s), {} to sell, {} trade protected, in {} ms",
            logbus::label(&steam_id),
            items.len(),
            items.iter().filter(|i| i.marketable).count(),
            items.iter().filter(|i| i.protected).count(),
            started.elapsed().as_millis()
        ),
    );
    Ok(items)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cs2Sell {
    pub asset_id: String,
    pub hash_name: String,
    /// What the buyer pays, in minor units.
    pub buyer_cents: u32,
}

#[tauri::command]
pub async fn cs2_sell(app: State<'_, App>, steam_id: String, items: Vec<Cs2Sell>) -> CmdResult<Vec<SellOutcome>> {
    app.market_cancel.store(false, Ordering::SeqCst);
    let _job = app.market_job();
    let started = Instant::now();
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let delay = app.settings().market.delay_ms.max(500);
    let wallet = wallet_for_prices(&app, &ws, &steam_id).await;
    let total = items.len();
    let mut out = Vec::new();
    let mut to_confirm = 0;
    for (done, item) in items.iter().enumerate() {
        if app.market_cancel.load(Ordering::SeqCst) {
            logbus::info("cs2", format!("Listing cancelled after {done} of {total}"));
            break;
        }
        let seller = wallet.seller_receives(item.buyer_cents);
        if seller == 0 {
            logbus::warn("cs2", format!("{}: {} is too low a price to list at", item.hash_name, amount(item.buyer_cents as u64, wallet.currency)));
            out.push(SellOutcome { asset_id: item.asset_id.clone(), ok: false, needs_confirmation: false, message: "The price is too low to list at".into() });
            continue;
        }
        progress("sell", done, total, format!("Listing {}", item.hash_name));
        let mut ended = false;
        let r = market::sell_asset(&ws, cs2::APP, cs2::CONTEXT, &item.asset_id, seller).await.unwrap_or_else(|e| {
            ended = community::session_ended(&e);
            app.web_failed(&steam_id, &ws, &e);
            SellOutcome { asset_id: item.asset_id.clone(), ok: false, needs_confirmation: false, message: format!("{e:#}") }
        });
        if r.ok {
            to_confirm += r.needs_confirmation as usize;
            logbus::info(
                "cs2",
                format!(
                    "{}: listed for {} (you receive {}){}",
                    item.hash_name,
                    amount(item.buyer_cents as u64, wallet.currency),
                    amount(seller as u64, wallet.currency),
                    if r.needs_confirmation { ", to confirm in the Steam mobile app" } else { "" }
                ),
            );
        } else {
            logbus::warn("cs2", format!("{}: not listed. {}", item.hash_name, r.message));
        }
        out.push(r);
        if ended {
            logbus::warn("cs2", "Stopped listing: Steam no longer takes this web session. Try again: a new one is made");
            break;
        }
        if done + 1 < total {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    }
    progress("sell", total, total, "Done");
    logbus::info(
        "cs2",
        format!(
            "Listed {} of {total} CS2 item(s) in {} s{}",
            out.iter().filter(|o| o.ok).count(),
            started.elapsed().as_secs(),
            if to_confirm > 0 { format!("; {to_confirm} to confirm in the Steam mobile app") } else { String::new() }
        ),
    );
    Ok(out)
}

// ---------------------------------------------------------- trade offers ---

#[tauri::command]
pub async fn trade_offers(app: State<'_, App>, steam_id: String) -> CmdResult<Offers> {
    let started = Instant::now();
    let now = chrono::Utc::now().timestamp();
    let offers = app.with_web(&steam_id, |ws| async move { trade::offers(&ws, now).await }).await.map_err(|e| {
        let why = told(e);
        logbus::warn("trade", format!("Trade offers of {}: {why}", logbus::label(&steam_id)));
        why
    })?;
    logbus::debug(
        "trade",
        format!("Trade offers of {}: {} received, {} sent, in {} ms", logbus::label(&steam_id), offers.received.len(), offers.sent.len(), started.elapsed().as_millis()),
    );
    Ok(offers)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartnerItems {
    /// The other player's Steam ID, read off the link.
    pub partner: String,
    pub items: Vec<Cs2Item>,
}

#[tauri::command]
pub async fn trade_partner_items(app: State<'_, App>, steam_id: String, link: String) -> CmdResult<PartnerItems> {
    let link = trade::parse_trade_link(&link).map_err(anyhow_err)?;
    let partner = link.steam_id();
    if partner == steam_id {
        return Err("That is this account's own trade link".into());
    }
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let items = trade::partner_inventory(&ws, &link).await.map_err(|e| {
        app.web_failed(&steam_id, &ws, &e);
        let why = told(e);
        logbus::warn("trade", format!("Items of {}: {why}", player(&partner)));
        why
    })?;
    logbus::debug("trade", format!("Items of {} that can be traded: {}", player(&partner), items.len()));
    Ok(PartnerItems { partner, items })
}

/// Only CS2 items that can be traded go into an offer made here.
fn cs2_assets(list: &[AssetRef]) -> Result<(), String> {
    for a in list {
        if a.app != cs2::APP || a.context != cs2::CONTEXT || a.asset_id.is_empty() || !a.asset_id.chars().all(|c| c.is_ascii_digit()) {
            return Err("An offer made here holds CS2 items only".into());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn trade_send(
    app: State<'_, App>,
    steam_id: String,
    link: String,
    give: Vec<AssetRef>,
    receive: Vec<AssetRef>,
    message: String,
) -> CmdResult<TradeOutcome> {
    let link = trade::parse_trade_link(&link).map_err(anyhow_err)?;
    cs2_assets(&give)?;
    cs2_assets(&receive)?;
    if give.is_empty() && receive.is_empty() {
        return Err("An offer needs at least one item".into());
    }
    let partner = link.steam_id();
    let what = format!("to {}: {} item(s) for {} item(s)", player(&partner), give.len(), receive.len());
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    logbus::info("trade", format!("Sending a trade offer {what}"));
    let sent = trade::send(&ws, &link, &give, &receive, &message).await.map_err(|e| {
        app.web_failed(&steam_id, &ws, &e);
        let why = told(e);
        logbus::warn("trade", format!("Trade offer {what}: Steam wasn't reached, or didn't answer ({why}); look at the sent offers before trying again"));
        why
    })?;
    if sent.ok {
        logbus::info(
            "trade",
            format!(
                "Trade offer sent {what}{}{}",
                sent.offer_id.as_deref().map(|id| format!(" (offer {})", short(id))).unwrap_or_default(),
                if sent.needs_confirmation { if sent.by_email { "; to confirm by e-mail" } else { "; to confirm in the Steam mobile app" } } else { "" }
            ),
        );
    } else {
        logbus::warn("trade", format!("Trade offer {what}: Steam refused. {}", sent.message));
    }
    Ok(sent)
}

#[tauri::command]
pub async fn trade_accept(app: State<'_, App>, steam_id: String, offer_id: String, partner: String) -> CmdResult<TradeOutcome> {
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let what = format!("offer {} from {}", short(&offer_id), player(&partner));
    logbus::info("trade", format!("Accepting {what}"));
    let done = trade::accept(&ws, &offer_id, &partner).await.map_err(|e| {
        app.web_failed(&steam_id, &ws, &e);
        let why = told(e);
        logbus::warn("trade", format!("Accepting {what}: Steam wasn't reached, or didn't answer ({why}); look at the offer before trying again"));
        why
    })?;
    if done.ok {
        logbus::info(
            "trade",
            format!("Accepted {what}{}", if done.needs_confirmation { if done.by_email { "; to confirm by e-mail" } else { "; to confirm in the Steam mobile app" } } else { "" }),
        );
    } else {
        logbus::warn("trade", format!("Accepting {what}: Steam refused. {}", done.message));
    }
    Ok(done)
}

#[tauri::command]
pub async fn trade_withdraw(app: State<'_, App>, steam_id: String, offer_id: String, how: String) -> CmdResult<()> {
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let (doing, did) = if how == "cancel" { ("Cancelling", "Cancelled") } else { ("Declining", "Declined") };
    match trade::withdraw(&ws, &offer_id, &how).await {
        Ok(()) => {
            logbus::info("trade", format!("{did} offer {}", short(&offer_id)));
            Ok(())
        }
        Err(e) => {
            app.web_failed(&steam_id, &ws, &e);
            let why = told(e);
            logbus::warn("trade", format!("{doing} offer {}: {why}", short(&offer_id)));
            Err(why)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buyer(balance: Option<u64>) -> Buyer {
        Buyer { currency: 31, balance, country: "SA".into(), ..Default::default() }
    }

    #[test]
    fn a_purchase_that_cant_work_isnt_sent_to_steam() {
        // Enough in the wallet, in its own currency: nothing in the way.
        assert_eq!(purchase_objection(&buyer(Some(10_000)), 31, 8607), None);
        assert_eq!(purchase_objection(&buyer(None), 31, 8607), None, "a wallet Steam gave no balance for is left to Steam");
        let other = purchase_objection(&buyer(Some(10_000)), 1, 8607).unwrap();
        assert!(other.starts_with("This price is in USD and the wallet is in SAR"), "{other}");
        assert_eq!(purchase_objection(&buyer(Some(8606)), 31, 8607).unwrap(), "The wallet has SAR 86.06 and this costs SAR 86.07");
        let mut no_address = buyer(Some(10_000));
        no_address.needs_billing = true;
        assert!(purchase_objection(&no_address, 31, 100).unwrap().contains("billing address"));
        no_address.has_billing = true;
        assert_eq!(purchase_objection(&no_address, 31, 100), None);
    }

    #[test]
    fn amounts_and_numbers_are_written_short_and_readable() {
        assert_eq!(amount(8607, 31), "SAR 86.07");
        assert_eq!(amount(5, 1), "USD 0.05");
        assert_eq!(short("5934539222390898406"), "…8406");
        assert_eq!(short("42"), "42");
        assert_eq!(player("76561198006409530"), "player …9530");
    }

    #[test]
    fn only_cs2_items_go_into_an_offer() {
        let ok = AssetRef { app: 730, context: "2".into(), asset_id: "100".into() };
        assert!(cs2_assets(std::slice::from_ref(&ok)).is_ok());
        assert!(cs2_assets(&[AssetRef { app: 753, ..ok.clone() }]).is_err());
        assert!(cs2_assets(&[AssetRef { context: "16".into(), ..ok.clone() }]).is_err(), "trade-protected items can't be traded on");
        assert!(cs2_assets(&[AssetRef { asset_id: "1; drop".into(), ..ok }]).is_err());
    }

    #[test]
    fn a_search_is_named_in_the_log_by_what_it_asks() {
        let mut q = Query { text: "redline".into(), sort: 3, direction: 1, ..Default::default() };
        q.filters.insert("Type".into(), vec!["CSGO_Type_Rifle".into()]);
        q.filters.insert("Exterior".into(), vec!["WearCategory2".into(), "WearCategory1".into()]);
        assert_eq!(search_words(&q), "\"redline\", 3 filter tag(s), cheapest first");
        assert_eq!(search_words(&Query::default()), "trending first");
    }
}
