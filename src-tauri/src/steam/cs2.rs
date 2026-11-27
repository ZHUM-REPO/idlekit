//! Counter-Strike 2 on the Steam Community Market, asked for the way Steam's
//! own market pages ask since November 2026: the search with its filters, an
//! item's listings (with wear, pattern and stickers), its order book and
//! price history, buying a listing, buy orders, the account's own listings
//! and orders, and the CS2 inventory for selling from.
//!
//! The answers these parsers read were looked at in a browser, without
//! signing in: the search, the listings, the order book, the price history
//! and the filter list are Steam's as they came. What only a signed-in
//! account gets (buying, buy orders, the wallet, the account's own listings)
//! is read as Steam's page code handles it, and hasn't been tried on Steam.
use super::community::{self, community_url, SessionEnded, WebSession};
use super::market::{self, money_cents, Wallet};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

/// Counter-Strike 2's app id.
pub const APP: u32 = 730;
/// The inventory CS2 items are in.
pub const CONTEXT: &str = "2";
/// Where items received in a trade wait for a week ("trade protected"):
/// they show in the inventory, and can't be sold or traded on until then.
pub const CONTEXT_PROTECTED: &str = "16";

/// How many search results Steam gives at most in one answer.
pub const SEARCH_MOST: usize = 100;
/// How many price levels of each side of an order book are passed on.
pub const LEVELS: usize = 40;

const ICONS: &str = "https://community.fastly.steamstatic.com/economy/image/";

/// A picture of an item from the name Steam gives it, at a size.
pub fn icon(hash: &str, size: u32) -> String {
    if hash.is_empty() {
        String::new()
    } else {
        format!("{ICONS}{hash}/{size}fx{size}f")
    }
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// A whole number, whether Steam wrote it as a number or in quotes.
fn whole(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn count(v: Option<&Value>) -> u32 {
    whole(v).map_or(0, |n| n.min(u32::MAX as u64) as u32)
}

/// A number with a fraction, written either way.
fn fraction(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Yes or no, as `true`, `1` or `"1"`.
fn yes(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        Some(Value::String(s)) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// The answer of one of the market's queries, out of its wrapping
/// (`{"data": …}`). `None` when Steam sent `null`, which it does now and
/// then for a query it answers a moment later.
fn query_data(text: &str, what: &str) -> Result<Option<Value>> {
    let v: Value = serde_json::from_str(text).map_err(|_| anyhow!("Steam answered with a page instead of {what}"))?;
    match v {
        Value::Object(mut map) if map.contains_key("data") => Ok(map.remove("data").filter(|d| !d.is_null())),
        Value::Null => Ok(None),
        other => Ok(Some(other)),
    }
}

fn query_url(path: &str, name: &str, args: &Value) -> String {
    community_url(&format!("{path}?q={name}&qp={}", urlencoding::encode(&args.to_string())))
}

// --------------------------------------------------------------- filters ---

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub name: String,
    /// How many items on the market carry it.
    pub count: u64,
    pub color: String,
}

/// One of the things the search can be narrowed by (Type, Weapon, Exterior…).
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Facet {
    pub id: String,
    pub name: String,
    pub tags: Vec<Tag>,
}

/// Reads Steam's list of filters for a game: each with the tags it has, how
/// they are called and how many items carry them.
pub fn parse_facets(text_: &str) -> Result<Vec<Facet>> {
    let v: Value = serde_json::from_str(text_).map_err(|_| anyhow!("Steam answered with a page instead of the market's filters"))?;
    let facets = v
        .get("facets")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("Steam's list of filters isn't in the expected form"))?;
    let mut out: Vec<Facet> = facets
        .iter()
        .filter_map(|(id, f)| {
            let tags = f.get("tags")?.as_object()?;
            let mut tags: Vec<Tag> = tags
                .iter()
                .map(|(tag, t)| Tag {
                    id: tag.clone(),
                    name: t.get("localized_name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or(tag).to_string(),
                    count: whole(t.get("matches")).unwrap_or(0),
                    color: text(t, "color"),
                })
                .collect();
            tags.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
            Some(Facet { id: id.clone(), name: f.get("localized_name").and_then(Value::as_str).unwrap_or(id).to_string(), tags })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

pub async fn facets(ws: &WebSession) -> Result<Vec<Facet>> {
    let text = ws.get_text(&community_url(&format!("/market/appfacets/{APP}"))).await?;
    parse_facets(&text)
}

// ---------------------------------------------------------------- search ---

/// What the window asks the market's search for.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Query {
    pub text: String,
    /// Filter → the tags picked in it ("Type" → ["CSGO_Type_WeaponCase"]).
    pub filters: BTreeMap<String, Vec<String>>,
    /// Steam's sort column: 1 name, 2 quantity, 3 price, 4 popularity, 5 trending.
    pub sort: u32,
    /// 1 lowest first, 2 highest first.
    pub direction: u32,
    pub start: usize,
    pub count: usize,
    /// Look in the items' descriptions too, not only their names.
    pub descriptions: bool,
    /// A price range, in the wallet currency's minor units.
    pub price_min: Option<u32>,
    pub price_max: Option<u32>,
}

/// A filter or tag name as Steam writes them: letters, digits, "_" and "-".
fn plain_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 96 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Query {
    /// The query as Steam's search is asked it, and which of the two
    /// searches takes it.
    pub fn ask(&self, currency: u32) -> (&'static str, Value) {
        let filters: BTreeMap<&str, Vec<&str>> = self
            .filters
            .iter()
            .filter(|(facet, tags)| plain_name(facet) && !tags.is_empty())
            .take(16)
            .map(|(facet, tags)| (facet.as_str(), tags.iter().filter(|t| plain_name(t)).take(64).map(String::as_str).collect()))
            .collect();
        let mut price = json!({ "eCurrency": currency });
        if let Some(min) = self.price_min.filter(|&m| m > 0) {
            price["unMin"] = json!(min);
        }
        if let Some(max) = self.price_max.filter(|&m| m > 0) {
            price["unMax"] = json!(max);
        }
        let mut ask = json!({
            "appid": APP,
            "filters": filters,
            "accessoryFilters": {},
            "price": price,
            "start": self.start,
            "count": self.count.clamp(1, SEARCH_MOST),
            "sort": if (1..=5).contains(&self.sort) { self.sort } else { 5 },
            "direction": if self.direction == 1 { 1 } else { 2 },
        });
        let words = self.text.trim();
        if !words.is_empty() {
            ask["strQuery"] = json!(words.chars().take(200).collect::<String>());
        }
        if self.descriptions && !words.is_empty() {
            ask["bSearchDescriptions"] = json!(true);
            ("SearchDescriptions", ask)
        } else {
            ("Search", ask)
        }
    }
}

/// One result of the search: an item, or a group of items that differ in
/// wear or in being StatTrak or Souvenir, with the cheapest of them.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    /// What the item's page is asked for by: the group's id when the item
    /// comes in several kinds, else its market name.
    pub id: String,
    pub hash_name: String,
    pub name: String,
    pub kind: String,
    pub icon_url: String,
    pub color: String,
    /// Sold by the piece from one order book (cases, keys, stickers), not as
    /// listings that each differ (skins).
    pub commodity: bool,
    pub grouped: bool,
    pub listings: u32,
    pub currency: u32,
    /// The cheapest listing as the buyer pays it, in minor units.
    pub price: Option<u32>,
    /// The same, as Steam writes it out.
    pub price_text: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hits {
    pub total: usize,
    pub start: usize,
    pub results: Vec<Hit>,
    /// Whether descriptions were searched as asked: false when Steam gave
    /// that search no answer and only names were searched.
    pub in_descriptions: bool,
}

/// Reads an answer of the market's search for a game's items. `None` when
/// Steam sent no answer (`null`).
pub fn parse_hits(text_: &str) -> Result<Option<Hits>> {
    let Some(data) = query_data(text_, "search results")? else { return Ok(None) };
    let results = data
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Steam's search answer isn't in the expected form"))?;
    let grouping = whole(data.get("grouping")).unwrap_or(0) == 1;
    let results: Vec<Hit> = results
        .iter()
        .filter_map(|r| {
            let d = r.get("asset_description").filter(|d| d.is_object())?;
            let hash_name = r.get("strHash").and_then(Value::as_str).map(str::to_string).filter(|h| !h.is_empty()).or_else(|| Some(text(d, "market_hash_name")).filter(|h| !h.is_empty()))?;
            let group_id = text(d, "market_bucket_group_id");
            let group_name = text(d, "market_bucket_group_name");
            let commodity = yes(d.get("commodity"));
            // A group of one (a case, a key) is asked for like any item.
            let grouped = grouping && !commodity && !group_id.is_empty() && !group_name.is_empty();
            let own_name = Some(text(d, "market_name")).filter(|n| !n.is_empty()).unwrap_or_else(|| text(d, "name"));
            let price_text = text(r, "strMinSellSubtotal");
            Some(Hit {
                id: if group_id.is_empty() { hash_name.clone() } else { group_id },
                name: if grouped { group_name } else if own_name.is_empty() { hash_name.clone() } else { own_name },
                hash_name,
                kind: text(d, "type"),
                icon_url: icon(&text(d, "icon_url"), 128),
                color: text(d, "name_color"),
                commodity,
                grouped,
                listings: count(r.get("cSellOrders")),
                currency: count(r.get("eCurrency")),
                price: money_cents(&price_text).filter(|&p| p > 0),
                price_text,
            })
        })
        .collect();
    Ok(Some(Hits {
        total: whole(data.get("total_count")).map_or(results.len(), |n| n as usize),
        start: whole(data.get("start")).unwrap_or(0) as usize,
        results,
        in_descriptions: false,
    }))
}

/// One answer of the market's search for CS2 items. A search within
/// descriptions that Steam gives no answer to is asked again by name only.
/// "Too many requests" comes back as a [`SteamBusy`](community::SteamBusy) error.
pub async fn search(ws: &WebSession, q: &Query, currency: u32) -> Result<Hits> {
    let (name, ask) = q.ask(currency);
    let get = |name: &'static str, ask: Value| async move {
        let url = query_url("/market/actions", name, &json!([ask]));
        let text = ws.get_query_or_busy(&url).await.map_err(|e| market::plain_refusal(e, "market search"))?;
        parse_hits(&text)
    };
    if name == "SearchDescriptions" {
        if let Some(mut hits) = get(name, ask).await? {
            hits.in_descriptions = true;
            return Ok(hits);
        }
        let plain = Query { descriptions: false, ..q.clone() };
        let (name, ask) = plain.ask(currency);
        return get(name, ask).await?.ok_or_else(|| anyhow!("Steam's market search gave no answer"));
    }
    match get(name, ask.clone()).await? {
        Some(hits) => Ok(hits),
        None => {
            // "null" now and then: once more, a moment later.
            tokio::time::sleep(Duration::from_millis(1500)).await;
            get(name, ask).await?.ok_or_else(|| anyhow!("Steam's market search gave no answer"))
        }
    }
}

/// A name the search box may complete to.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub id: String,
    pub name: String,
    pub icon_url: String,
}

pub fn parse_suggestions(text_: &str) -> Vec<Suggestion> {
    let v: Value = serde_json::from_str(text_).unwrap_or(Value::Null);
    v.get("results")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|r| whole(r.get("app_id")).is_none_or(|a| a == APP as u64))
                .filter_map(|r| {
                    let id = text(r, "market_hash_name");
                    let name = text(r, "market_name");
                    (!id.is_empty() && !name.is_empty()).then(|| Suggestion { id, name, icon_url: text(r, "icon_url") })
                })
                .take(10)
                .collect()
        })
        .unwrap_or_default()
}

/// What Steam's search box offers for a few typed letters.
pub async fn suggestions(ws: &WebSession, typed: &str) -> Result<Vec<Suggestion>> {
    let url = community_url(&format!("/market/searchsuggestionsresults?q={}&appid={APP}", urlencoding::encode(typed.trim())));
    Ok(parse_suggestions(&ws.get_text_or_busy(&url).await?))
}

// -------------------------------------------------------------- listings ---

/// A sticker or charm on a listed item.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sticker {
    pub name: String,
    pub kind: String,
    pub icon_url: String,
    /// How scraped a sticker is, 0 to 1, where Steam says.
    pub wear: Option<f64>,
}

/// One item for sale. Prices are in `currency`'s minor units.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListingRow {
    pub listing_id: String,
    pub asset_id: String,
    pub hash_name: String,
    /// Which of the group's kinds it is ("Field-Tested"), where it has one.
    pub variant: String,
    pub name: String,
    pub color: String,
    /// What the seller gets.
    pub price: u32,
    /// Steam's and the game's fees on top.
    pub fee: u32,
    /// What the buyer pays: the two together.
    pub total: u32,
    pub currency: u32,
    pub price_text: String,
    /// Wear, 0 (new) to 1.
    pub float: Option<f64>,
    /// The pattern's number.
    pub pattern: Option<u32>,
    /// Opens the item in the game.
    pub inspect: Option<String>,
    pub stickers: Vec<Sticker>,
    /// A picture of this very item, where Steam made one.
    pub image: Option<String>,
    pub icon_url: String,
    /// The account's own listing.
    pub mine: bool,
}

/// How many of an item's listings carry a tag (Field-Tested: 1395).
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TagCount {
    pub facet: String,
    pub facet_name: String,
    pub tag: String,
    pub tag_name: String,
    pub color: String,
    pub listings: u32,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemPage {
    pub total: usize,
    pub start: usize,
    pub more: bool,
    pub listings: Vec<ListingRow>,
    pub facets: Vec<TagCount>,
}

/// A property of an asset by its number: 1 the pattern, 2 the wear, 6 the
/// code its "Inspect in Game" link needs.
fn property(asset: &Value, id: u64) -> Option<&Value> {
    asset.get("asset_properties")?.as_array()?.iter().find(|p| whole(p.get("propertyid")) == Some(id))
}

/// The "Inspect in Game" link of a listed item: Steam's template with the
/// places it leaves open filled in.
fn inspect_link(description: &Value, asset: &Value, listing_id: &str) -> Option<String> {
    let template = ["actions", "market_actions"]
        .iter()
        .filter_map(|key| description.get(*key)?.as_array()?.first()?.get("link")?.as_str())
        .find(|link| link.starts_with("steam://"))?;
    let code = property(asset, 6).and_then(|p| p.get("string_value")).and_then(Value::as_str).unwrap_or("");
    if template.contains("%propid:6%") && code.is_empty() {
        return None;
    }
    Some(
        template
            .replace("%propid:6%", code)
            .replace("%listingid%", listing_id)
            .replace("%assetid%", asset.get("id").and_then(Value::as_str).unwrap_or("")),
    )
}

fn parse_listing(l: &Value) -> Option<ListingRow> {
    let listing_id = l.get("listingid").and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_u64().map(|n| n.to_string())))?;
    let d = l.get("description").filter(|d| d.is_object())?;
    let asset = l.get("asset").unwrap_or(&Value::Null);
    let per_unit = |unit: &str, all: &str| count(l.get(unit)).max(if l.get(unit).is_some() { 0 } else { count(l.get(all)) });
    let price = per_unit("unPricePerUnit", "unPrice");
    let fee = per_unit("unFeePerUnit", "unFee");
    if price == 0 {
        return None;
    }
    let stickers = asset
        .get("asset_accessories")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let ad = a.get("description")?;
                    let name = text(ad, "name");
                    (!name.is_empty()).then(|| Sticker {
                        name,
                        kind: text(ad, "type"),
                        icon_url: icon(&text(ad, "icon_url"), 64),
                        wear: a
                            .get("parent_relationship_properties")
                            .and_then(Value::as_array)
                            .and_then(|props| props.iter().find(|p| whole(p.get("propertyid")) == Some(4)))
                            .and_then(|p| fraction(p.get("float_value"))),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ListingRow {
        asset_id: asset.get("id").or_else(|| asset.get("assetid")).and_then(Value::as_str).unwrap_or("").to_string(),
        hash_name: text(d, "market_hash_name"),
        variant: text(d, "market_name_inside_group"),
        name: Some(text(d, "market_name")).filter(|n| !n.is_empty()).unwrap_or_else(|| text(d, "name")),
        color: text(d, "name_color"),
        price,
        fee,
        total: price.saturating_add(fee),
        currency: count(l.get("eCurrency")),
        price_text: text(l, "strSubtotal").replace('\u{a0}', " "),
        float: property(asset, 2).and_then(|p| fraction(p.get("float_value"))).filter(|f| (0.0..=1.0).contains(f)),
        pattern: property(asset, 1).and_then(|p| whole(p.get("int_value"))).map(|n| n.min(u32::MAX as u64) as u32),
        inspect: inspect_link(d, asset, &listing_id),
        stickers,
        image: l
            .get("enhanced_appearances")
            .and_then(Value::as_array)
            .and_then(|list| list.iter().find_map(|e| e.get("url")?.as_str()))
            .filter(|u| u.starts_with("https://") && !u.ends_with('…'))
            .map(str::to_string),
        icon_url: icon(&text(d, "icon_url"), 128),
        mine: yes(l.get("bMine")),
        listing_id,
    })
}

/// Reads one answer of an item's listings. `None` when Steam sent `null`.
pub fn parse_item(text_: &str) -> Result<Option<ItemPage>> {
    let Some(data) = query_data(text_, "the item's listings")? else { return Ok(None) };
    let listings = data.get("listings").and_then(Value::as_array).ok_or_else(|| anyhow!("Steam's listings answer isn't in the expected form"))?;
    let facets = data
        .get("facets")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|f| {
                    let tag = f.get("tag")?;
                    let (facet, id) = (text(tag, "category"), text(tag, "internal_name"));
                    (!facet.is_empty() && !id.is_empty()).then(|| TagCount {
                        facet_name: Some(text(tag, "localized_category_name")).filter(|n| !n.is_empty()).unwrap_or_else(|| facet.clone()),
                        tag_name: Some(text(tag, "localized_tag_name")).filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone()),
                        facet,
                        tag: id,
                        color: text(tag, "color"),
                        listings: count(f.get("listings")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let listings: Vec<ListingRow> = listings.iter().filter_map(parse_listing).collect();
    Ok(Some(ItemPage {
        total: whole(data.get("total_count")).map_or(listings.len(), |n| n as usize),
        start: whole(data.get("start")).unwrap_or(0) as usize,
        more: yes(data.get("more")),
        listings,
        facets,
    }))
}

/// How often an item's listings are asked for again when Steam sends `null`,
/// and how long each wait is.
const NULL_WAITS: [u64; 2] = [1500, 3000];

/// An item's listings from `start` on, cheapest first, twenty at a time:
/// `id` is what the search gave the item, and `filters` narrows a group to
/// some of its kinds (Exterior → Field-Tested). Steam now and then answers
/// `null` and the listings a moment later, so that is waited out twice.
pub async fn item(ws: &WebSession, id: &str, filters: &BTreeMap<String, Vec<String>>, start: usize) -> Result<ItemPage> {
    let filters: BTreeMap<&str, Vec<&str>> = filters
        .iter()
        .filter(|(facet, tags)| plain_name(facet) && !tags.is_empty())
        .map(|(facet, tags)| (facet.as_str(), tags.iter().filter(|t| plain_name(t)).map(String::as_str).collect()))
        .collect();
    let ask = json!([{ "appid": APP, "strItemName": id, "filters": filters, "accessoryFilters": {}, "start": start }]);
    let url = query_url("/market/actions", "QueryListingsForItem", &ask);
    let mut tries = 0;
    loop {
        let text = ws.get_query_or_busy(&url).await.map_err(|e| market::plain_refusal(e, "listings lookup"))?;
        if let Some(page) = parse_item(&text)? {
            return Ok(page);
        }
        let Some(&wait) = NULL_WAITS.get(tries) else {
            bail!("Steam gave no answer for this item's listings, three times in a row. Try again in a moment");
        };
        crate::logbus::debug("cs2", format!("Steam answered the listings with nothing; asking again in {:.1} s ({} of {})", wait as f64 / 1000.0, tries + 1, NULL_WAITS.len()));
        tokio::time::sleep(Duration::from_millis(wait)).await;
        tries += 1;
    }
}

// ----------------------------------------------------------- description ---

/// What an item is, for the head of its page.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct About {
    pub hash_name: String,
    pub name: String,
    pub kind: String,
    pub icon_url: String,
    pub color: String,
    pub commodity: bool,
    pub group_id: String,
    pub group_name: String,
    pub variant: String,
    /// The item's description, line by line, without markup.
    pub lines: Vec<String>,
    /// Days a bought item can't be traded or sold on.
    pub hold_days: u32,
}

/// A description line as plain text: tags out, the few entities Steam uses in.
fn plain(html: &str) -> String {
    let no_tags = re!(r"<[^>]*>").replace_all(html, "");
    no_tags.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&nbsp;", " ").trim().to_string()
}

pub fn parse_about(text_: &str) -> Result<Option<About>> {
    let Some(d) = query_data(text_, "the item's description")? else { return Ok(None) };
    let hash_name = text(&d, "market_hash_name");
    if hash_name.is_empty() {
        return Ok(None);
    }
    let lines = d
        .get("descriptions")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|l| l.get("value")?.as_str()).map(plain).filter(|l| !l.is_empty()).take(12).collect())
        .unwrap_or_default();
    Ok(Some(About {
        name: Some(text(&d, "market_name")).filter(|n| !n.is_empty()).unwrap_or_else(|| text(&d, "name")),
        kind: text(&d, "type"),
        icon_url: icon(&text(&d, "icon_url"), 256),
        color: text(&d, "name_color"),
        commodity: yes(d.get("commodity")),
        group_id: text(&d, "market_bucket_group_id"),
        group_name: text(&d, "market_bucket_group_name"),
        variant: text(&d, "market_name_inside_group"),
        lines,
        hold_days: count(d.get("market_tradable_restriction")).max(count(d.get("market_marketable_restriction"))),
        hash_name,
    }))
}

pub async fn about(ws: &WebSession, hash_name: &str) -> Result<Option<About>> {
    let url = query_url("/market/actions", "QueryDescription", &json!([APP, hash_name]));
    let text = ws.get_query_or_busy(&url).await.map_err(|e| market::plain_refusal(e, "description lookup"))?;
    parse_about(&text)
}

// ------------------------------------------------------------ order book ---

#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Level {
    /// What a buyer pays (or offers) for one, in minor units.
    pub price: u32,
    pub quantity: u32,
}

/// An item's order book with its price levels: what is for sale at which
/// price, and what buyers offer.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Depth {
    pub currency: u32,
    pub lowest_sell: Option<u32>,
    pub highest_buy: Option<u32>,
    pub sell_count: u32,
    pub buy_count: u32,
    /// Cheapest first.
    pub sells: Vec<Level>,
    /// Highest offer first.
    pub buys: Vec<Level>,
    /// How many more price levels each side has than the ones given.
    pub more_sells: usize,
    pub more_buys: usize,
}

/// Price levels out of Steam's flat list of price, quantity, price, quantity…
fn levels(v: Option<&Value>) -> (Vec<Level>, usize) {
    let flat: Vec<u32> = v.and_then(Value::as_array).map(|a| a.iter().map(|n| count(Some(n))).collect()).unwrap_or_default();
    let all = flat.len() / 2;
    let shown = flat.chunks_exact(2).take(LEVELS).filter(|pair| pair[0] > 0 && pair[1] > 0).map(|pair| Level { price: pair[0], quantity: pair[1] }).collect();
    (shown, all.saturating_sub(LEVELS))
}

/// Reads an order book with its levels. `None` when Steam has no market for
/// the item.
pub fn parse_depth(text_: &str) -> Result<Option<Depth>> {
    let Some(book) = market::parse_order_book(text_)? else { return Ok(None) };
    let v: Value = serde_json::from_str(text_).unwrap_or(Value::Null);
    let answer = if v.get("success").is_some() { &v } else { v.get("data").unwrap_or(&Value::Null) };
    let data = answer.get("data").unwrap_or(&Value::Null);
    let (sells, more_sells) = levels(data.get("rgCompactSellOrders"));
    let (buys, more_buys) = levels(data.get("rgCompactBuyOrders"));
    Ok(Some(Depth {
        currency: book.currency,
        lowest_sell: book.lowest_sell,
        highest_buy: book.highest_buy,
        sell_count: book.sell_listings,
        buy_count: count(data.get("cBuyOrders")),
        sells,
        buys,
        more_sells,
        more_buys,
    }))
}

/// An item's order book by its market name. "Too many requests" comes back
/// as a [`SteamBusy`](community::SteamBusy) error.
pub async fn depth(ws: &WebSession, hash_name: &str) -> Result<Option<Depth>> {
    let url = query_url("/market/orderbook", "Load", &json!([APP, hash_name]));
    let text = ws.get_query_or_busy(&url).await.map_err(|e| market::plain_refusal(e, "order book lookup"))?;
    parse_depth(&text)
}

// --------------------------------------------------------- price history ---

/// What an item sold for over time: per point the time (seconds since
/// 1970), the median price in minor units and how many were sold.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub currency: u32,
    pub points: Vec<(i64, u32, u32)>,
}

pub fn parse_history(text_: &str) -> Result<Option<History>> {
    let Some(data) = query_data(text_, "the price history")? else { return Ok(None) };
    let Some(prices) = data.get("prices").and_then(Value::as_array) else { return Ok(None) };
    let mut points: Vec<(i64, u32, u32)> = prices
        .iter()
        .filter_map(|p| {
            let time = p.get("time").and_then(Value::as_i64).filter(|&t| t > 0)?;
            let median = fraction(p.get("price_median")).filter(|m| m.is_finite() && *m > 0.0 && *m < 4e7)?;
            Some((time, (median * 100.0).round() as u32, count(p.get("purchases"))))
        })
        .collect();
    points.sort_by_key(|p| p.0);
    Ok(Some(History { currency: count(data.get("ecurrency").or_else(|| data.get("eCurrency"))), points }))
}

pub async fn history(ws: &WebSession, hash_name: &str) -> Result<Option<History>> {
    let url = query_url("/market/actions", "QueryPriceHistory", &json!([APP, hash_name]));
    let text = ws.get_query_or_busy(&url).await.map_err(|e| market::plain_refusal(e, "price history lookup"))?;
    parse_history(&text)
}

// ---------------------------------------------------------------- buying ---

/// A billing address as Steam has it saved. It only ever goes back to Steam
/// with a purchase, where Steam asks for one: never to the window or the log.
#[derive(Clone, Default, PartialEq)]
pub struct Billing {
    pub first_name: String,
    pub last_name: String,
    pub address: String,
    pub address_two: String,
    pub city: String,
    pub state: String,
    pub postal_code: String,
}

impl std::fmt::Debug for Billing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Billing(…)")
    }
}

/// What Steam says about the account as a buyer: its wallet, and what a
/// purchase needs.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Buyer {
    pub currency: u32,
    /// What is in the wallet, in minor units.
    pub balance: Option<u64>,
    pub country: String,
    /// Steam wants a billing address with purchases from this country.
    pub needs_billing: bool,
    /// Whether one is saved with Steam to send along.
    pub has_billing: bool,
    /// How purchases are confirmed: 0 not at all, 1 by e-mail, 2 in the Steam mobile app.
    pub confirmation: u32,
    /// A tax added on the fees, in percent; 0 where there is none.
    pub tax_percent: f64,
    #[serde(skip)]
    pub billing: Billing,
}

impl Buyer {
    /// The tax Steam adds on a fee, the way its purchase dialog works it out.
    pub fn tax_on(&self, fee: u32) -> u32 {
        if self.tax_percent <= 0.0 || fee <= 2 {
            0
        } else {
            (fee as f64 * self.tax_percent / 100.0 + 0.5).floor() as u32
        }
    }
}

/// Reads Steam's answer about the account as a buyer. An answer that says
/// "no" is what a visitor who isn't signed in gets.
pub fn parse_buyer(text_: &str) -> Result<Buyer> {
    let v: Value = serde_json::from_str(text_).map_err(|_| anyhow!("Steam answered with a page instead of the wallet"))?;
    if v.get("success").is_some_and(|s| !yes(Some(s))) {
        return Err(SessionEnded.into());
    }
    let wallet = v.get("wallet_info").unwrap_or(&Value::Null);
    let currency = count(wallet.get("wallet_currency"));
    if currency == 0 {
        bail!("Steam's answer about the wallet doesn't say which currency it is in");
    }
    let tax = v.get("tax_rate").unwrap_or(&Value::Null);
    let address = v.get("billing_address").unwrap_or(&Value::Null);
    let billing = Billing {
        first_name: text(address, "firstname"),
        last_name: text(address, "lastname"),
        address: text(address, "address1"),
        address_two: text(address, "address2"),
        city: text(address, "city"),
        state: text(address, "state"),
        postal_code: text(address, "postcode"),
    };
    Ok(Buyer {
        currency,
        balance: whole(wallet.get("wallet_balance")),
        country: text(&v, "country_code"),
        needs_billing: yes(v.get("require_billing_info")),
        has_billing: !billing.first_name.is_empty() && !billing.address.is_empty(),
        confirmation: count(v.get("confirmation_type")),
        tax_percent: if yes(tax.get("tradefee_addtax")) { fraction(tax.get("tradefee_taxrate")).filter(|r| (0.0..100.0).contains(r)).unwrap_or(0.0) } else { 0.0 },
        billing,
    })
}

pub async fn buyer(ws: &WebSession) -> Result<Buyer> {
    let text = ws.get_text(&community_url("/market/userbillinginfo")).await?;
    parse_buyer(&text)
}

/// The listing a purchase is for, as the window saw it.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToBuy {
    pub listing_id: String,
    pub hash_name: String,
    /// What the seller gets and the fees on top, in minor units.
    pub price: u32,
    pub fee: u32,
    pub currency: u32,
}

/// How a purchase or an order ended.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bought {
    pub ok: bool,
    /// Steam waits for the purchase to be confirmed (in the mobile app or by
    /// e-mail): asking again with `confirmation_id` completes it.
    pub needs_confirmation: bool,
    pub confirmation_id: Option<String>,
    /// A buy order's number, when one was placed.
    pub order_id: Option<String>,
    pub message: String,
    /// The wallet after a purchase, where Steam says.
    pub balance: Option<u64>,
}

/// Reads Steam's answer to a purchase or to placing a buy order.
pub fn parse_bought(status: u16, text_: &str) -> Bought {
    let v: Value = serde_json::from_str(text_).unwrap_or(Value::Null);
    let confirmation_id = v
        .get("confirmation")
        .and_then(|c| c.get("confirmation_id"))
        .and_then(|id| id.as_str().map(str::to_string).or_else(|| id.as_u64().map(|n| n.to_string())))
        .filter(|id| !id.is_empty() && id != "0");
    let message = text(&v, "message");
    if yes(v.get("need_confirmation")) && confirmation_id.is_some() {
        return Bought { needs_confirmation: true, confirmation_id, message, ..Default::default() };
    }
    let success = v.get("success");
    // A purchase answers 200 with the wallet; an order answers "success": 1.
    let ok = (200..300).contains(&status) && v.is_object() && success.is_none_or(|s| yes(Some(s)));
    if ok {
        return Bought {
            ok: true,
            order_id: v.get("buy_orderid").and_then(|id| id.as_str().map(str::to_string).or_else(|| id.as_u64().map(|n| n.to_string()))),
            balance: v.get("wallet_info").and_then(|w| whole(w.get("wallet_balance"))),
            message,
            ..Default::default()
        };
    }
    Bought { message: if message.is_empty() { format!("Steam refused (HTTP {status})") } else { plain(&message) }, ..Default::default() }
}

fn billing_form<'a>(buyer: &'a Buyer, confirmation: &'a str) -> Vec<(&'static str, &'a str)> {
    // The address goes along only where Steam asks for one.
    let b = &buyer.billing;
    let part = |s: &'a String| if buyer.needs_billing { s.as_str() } else { "" };
    vec![
        ("first_name", part(&b.first_name)),
        ("last_name", part(&b.last_name)),
        ("billing_address", part(&b.address)),
        ("billing_address_two", part(&b.address_two)),
        ("billing_country", if buyer.needs_billing { buyer.country.as_str() } else { "" }),
        ("billing_city", part(&b.city)),
        ("billing_state", part(&b.state)),
        ("billing_postal_code", part(&b.postal_code)),
        ("confirmation", confirmation),
        ("save_my_address", "0"),
    ]
}

fn listing_page(hash_name: &str) -> String {
    format!("https://steamcommunity.com/market/listings/{APP}/{}", urlencoding::encode(hash_name))
}

/// Buys one listing with the wallet, for exactly the price the window saw:
/// Steam refuses when the listing has changed or is gone. Sent once, never
/// again by itself.
pub async fn buy_listing(ws: &WebSession, item: &ToBuy, buyer: &Buyer, confirmation: Option<&str>) -> Result<Bought> {
    if !item.listing_id.chars().all(|c| c.is_ascii_digit()) || item.listing_id.is_empty() {
        bail!("That isn't a listing's number");
    }
    let tax = buyer.tax_on(item.fee);
    let total = item.price as u64 + item.fee as u64 + tax as u64;
    let (currency, subtotal, fee, total, tax) = (buyer.currency.to_string(), item.price.to_string(), item.fee.to_string(), total.to_string(), tax.to_string());
    let mut form = vec![
        ("sessionid", ws.session_id.as_str()),
        ("currency", currency.as_str()),
        ("subtotal", subtotal.as_str()),
        ("fee", fee.as_str()),
        ("total", total.as_str()),
        ("tradefee_tax", tax.as_str()),
        ("quantity", "1"),
    ];
    form.extend(billing_form(buyer, confirmation.unwrap_or("0")));
    let url = community_url(&format!("/market/buylisting/{}", item.listing_id));
    let (status, text) = ws.post_form(&url, &listing_page(&item.hash_name), &form).await?;
    Ok(parse_bought(status, &text))
}

/// Places an order to buy `quantity` of an item at up to `price_each`
/// (what the buyer pays for one, in minor units). An order at or above the
/// cheapest listing buys at once: that is how cases and keys are bought.
pub async fn create_buy_order(
    ws: &WebSession,
    hash_name: &str,
    price_each: u32,
    quantity: u32,
    buyer: &Buyer,
    wallet: &Wallet,
    confirmation: Option<&str>,
) -> Result<Bought> {
    let total = price_each as u64 * quantity as u64;
    // The tax, where there is one, is on the fees that are part of the price.
    let fees_each = price_each.saturating_sub(wallet.seller_receives(price_each));
    let tax = if total < 3 || buyer.tax_percent <= 0.0 { 0 } else { (quantity as f64 * fees_each as f64 * buyer.tax_percent / 100.0 + 0.5).floor() as u64 };
    let (currency, app, total, tax, quantity) = (buyer.currency.to_string(), APP.to_string(), total.to_string(), tax.to_string(), quantity.to_string());
    let mut form = vec![
        ("sessionid", ws.session_id.as_str()),
        ("currency", currency.as_str()),
        ("appid", app.as_str()),
        ("market_hash_name", hash_name),
        ("price_total", total.as_str()),
        ("tradefee_tax", tax.as_str()),
        ("quantity", quantity.as_str()),
    ];
    form.extend(billing_form(buyer, confirmation.unwrap_or("0")));
    let (status, text) = ws.post_form(&community_url("/market/createbuyorder/"), &listing_page(hash_name), &form).await?;
    Ok(parse_bought(status, &text))
}

/// Where a buy order stands.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrderStatus {
    /// Still open: not all of it bought, not cancelled.
    pub active: bool,
    pub quantity: u32,
    pub remaining: u32,
    /// How many it has bought so far.
    pub bought: u32,
    /// What that cost, as Steam writes it out.
    pub paid_text: String,
}

pub fn parse_order_status(text_: &str) -> Result<OrderStatus> {
    let v: Value = serde_json::from_str(text_).map_err(|_| anyhow!("Steam answered with a page instead of the order's status"))?;
    if !yes(v.get("success")) {
        bail!("Steam doesn't know this buy order (it may be filled or cancelled)");
    }
    let quantity = count(v.get("quantity"));
    let remaining = count(v.get("quantity_remaining"));
    let bought = v.get("purchases").and_then(Value::as_array).map_or(0, |p| p.len() as u32).max(if yes(v.get("purchased")) { quantity.saturating_sub(remaining).max(1) } else { 0 });
    Ok(OrderStatus { active: yes(v.get("active")), quantity, remaining, bought, paid_text: plain(&text(&v, "purchase_amount_text")) })
}

pub async fn buy_order_status(ws: &WebSession, order_id: &str) -> Result<OrderStatus> {
    let url = community_url(&format!("/market/getbuyorderstatus/?sessionid={}&buy_orderid={}", ws.session_id, urlencoding::encode(order_id)));
    parse_order_status(&ws.get_text_or_busy(&url).await?)
}

pub async fn cancel_buy_order(ws: &WebSession, order_id: &str) -> Result<()> {
    let form = [("sessionid", ws.session_id.as_str()), ("buy_orderid", order_id)];
    let (status, text) = ws.post_form(&community_url("/market/cancelbuyorder/"), "https://steamcommunity.com/market/", &form).await?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if (200..300).contains(&status) && yes(v.get("success")) {
        return Ok(());
    }
    let why = Some(text_or(&v, "error", "message")).filter(|w| !w.is_empty()).unwrap_or_else(|| format!("HTTP {status}"));
    bail!("Steam didn't cancel the buy order: {why}")
}

fn text_or(v: &Value, first: &str, second: &str) -> String {
    Some(text(v, first)).filter(|t| !t.is_empty()).unwrap_or_else(|| text(v, second))
}

// ------------------------------------------------- own listings and orders --

/// One of the account's CS2 items for sale.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MyListing {
    pub listing_id: String,
    pub hash_name: String,
    pub name: String,
    pub icon_url: String,
    pub color: String,
    /// What a buyer pays, and what of it arrives, in minor units.
    pub buyer_pays: u32,
    pub you_receive: u32,
    pub currency: u32,
    /// "active", "pending" (to confirm in the mobile app) or "held".
    pub status: String,
    pub created: i64,
}

/// One of the account's open orders to buy.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MyOrder {
    pub order_id: String,
    pub hash_name: String,
    pub name: String,
    pub icon_url: String,
    pub color: String,
    /// What is offered for one, in minor units.
    pub price: u32,
    pub quantity: u32,
    pub remaining: u32,
    pub currency: u32,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Mine {
    pub listings: Vec<MyListing>,
    pub orders: Vec<MyOrder>,
}

/// Steam writes a listing's currency as 2000 plus its id.
fn listed_currency(v: Option<&Value>) -> u32 {
    let c = count(v);
    if c > 2000 { c - 2000 } else { c }
}

/// Reads the account's listings and buy orders, the CS2 ones of them.
pub fn parse_mine(text_: &str) -> Result<Mine> {
    let v: Value = serde_json::from_str(text_).map_err(|_| anyhow!("Steam answered with a page instead of your listings"))?;
    if !yes(v.get("success")) {
        return Err(SessionEnded.into());
    }
    let cs2 = |a: &Value| whole(a.get("appid")).is_none_or(|app| app == APP as u64);
    let mut listings = Vec::new();
    for (key, status) in [("listings", "active"), ("listings_to_confirm", "pending"), ("listings_on_hold", "held")] {
        for l in v.get(key).and_then(Value::as_array).into_iter().flatten() {
            let asset = l.get("asset").unwrap_or(&Value::Null);
            let Some(listing_id) = l.get("listingid").and_then(|id| id.as_str().map(str::to_string).or_else(|| id.as_u64().map(|n| n.to_string()))) else { continue };
            if !cs2(asset) {
                continue;
            }
            let (price, fee) = (count(l.get("price")), count(l.get("fee")));
            listings.push(MyListing {
                listing_id,
                hash_name: text(asset, "market_hash_name"),
                name: Some(text(asset, "market_name")).filter(|n| !n.is_empty()).unwrap_or_else(|| text(asset, "name")),
                icon_url: icon(&text(asset, "icon_url"), 96),
                color: text(asset, "name_color"),
                buyer_pays: price.saturating_add(fee),
                you_receive: price,
                currency: listed_currency(l.get("currencyid")),
                status: status.to_string(),
                created: l.get("time_created").and_then(Value::as_i64).unwrap_or(0),
            });
        }
    }
    let orders = v
        .get("buy_orders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|o| cs2(o))
        .filter_map(|o| {
            let order_id = o.get("buy_orderid").and_then(|id| id.as_str().map(str::to_string).or_else(|| id.as_u64().map(|n| n.to_string())))?;
            let d = o.get("description").unwrap_or(&Value::Null);
            let hash_name = text(o, "hash_name");
            Some(MyOrder {
                order_id,
                name: Some(text(d, "market_name")).filter(|n| !n.is_empty()).unwrap_or_else(|| hash_name.clone()),
                icon_url: icon(&text(d, "icon_url"), 96),
                color: text(d, "name_color"),
                price: count(o.get("price")),
                quantity: count(o.get("quantity")),
                remaining: count(o.get("quantity_remaining")),
                currency: listed_currency(o.get("wallet_currency")),
                hash_name,
            })
        })
        .collect();
    Ok(Mine { listings, orders })
}

pub async fn mine(ws: &WebSession) -> Result<Mine> {
    let text = ws.get_text(&community_url("/market/mylistings?norender=1&start=0&count=100")).await?;
    parse_mine(&text)
}

// ------------------------------------------------------------- inventory ---

/// One CS2 item in an inventory.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cs2Item {
    pub asset_id: String,
    /// Which inventory it is in: "2", or "16" while trade protected.
    pub context: String,
    pub class_id: String,
    pub instance_id: String,
    pub hash_name: String,
    pub name: String,
    pub kind: String,
    pub icon_url: String,
    pub color: String,
    pub exterior: String,
    pub marketable: bool,
    pub tradable: bool,
    /// Received in a trade in the last week: can't be sold or traded on yet.
    pub protected: bool,
    pub commodity: bool,
    pub float: Option<f64>,
    pub pattern: Option<u32>,
    /// What Steam notes on the item for its owner ("Tradable After …").
    pub notes: Vec<String>,
}

fn tag_name(d: &Value, category: &str) -> String {
    d.get("tags")
        .and_then(Value::as_array)
        .and_then(|tags| tags.iter().find(|t| t.get("category").and_then(Value::as_str) == Some(category)))
        .map(|t| text_or(t, "localized_tag_name", "name"))
        .unwrap_or_default()
}

fn item_from(d: &Value, asset_id: &str, context: &str) -> Option<Cs2Item> {
    let hash_name = text(d, "market_hash_name");
    let name = Some(text(d, "market_name")).filter(|n| !n.is_empty()).unwrap_or_else(|| text(d, "name"));
    if name.is_empty() && hash_name.is_empty() {
        return None;
    }
    let notes = d
        .get("owner_descriptions")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|l| l.get("value")?.as_str()).map(plain).filter(|l| !l.is_empty()).take(3).collect())
        .unwrap_or_default();
    let protected = context == CONTEXT_PROTECTED;
    Some(Cs2Item {
        asset_id: asset_id.to_string(),
        context: context.to_string(),
        class_id: text(d, "classid"),
        instance_id: Some(text(d, "instanceid")).filter(|i| !i.is_empty()).unwrap_or_else(|| "0".into()),
        name: if name.is_empty() { hash_name.clone() } else { name },
        hash_name,
        kind: text(d, "type"),
        icon_url: icon(&text(d, "icon_url"), 128),
        color: text(d, "name_color"),
        exterior: tag_name(d, "Exterior"),
        marketable: yes(d.get("marketable")) && !protected,
        tradable: yes(d.get("tradable")) && !protected,
        protected,
        commodity: yes(d.get("commodity")),
        float: None,
        pattern: None,
        notes,
    })
}

/// Adds one answer of an inventory to `out`: the form `/inventory/…` gives
/// (assets and descriptions side by side, with each asset's wear and pattern
/// in a list of their own). Says whether more follows, and from which asset.
pub fn parse_inventory_page(v: &Value, context: &str, out: &mut Vec<Cs2Item>) -> (bool, Option<String>) {
    let mut by_class: std::collections::HashMap<(&str, &str), &Value> = std::collections::HashMap::new();
    for d in v.get("descriptions").and_then(Value::as_array).into_iter().flatten() {
        let class = d.get("classid").and_then(Value::as_str).unwrap_or("");
        by_class.insert((class, d.get("instanceid").and_then(Value::as_str).unwrap_or("0")), d);
    }
    let mut props: std::collections::HashMap<&str, &Value> = std::collections::HashMap::new();
    for p in v.get("asset_properties").and_then(Value::as_array).into_iter().flatten() {
        if let Some(id) = p.get("assetid").and_then(Value::as_str) {
            props.insert(id, p);
        }
    }
    for a in v.get("assets").and_then(Value::as_array).into_iter().flatten() {
        let class = a.get("classid").and_then(Value::as_str).unwrap_or("");
        let instance = a.get("instanceid").and_then(Value::as_str).unwrap_or("0");
        let Some(d) = by_class.get(&(class, instance)) else { continue };
        let asset_id = a.get("assetid").and_then(Value::as_str).unwrap_or("");
        let Some(mut item) = item_from(d, asset_id, a.get("contextid").and_then(Value::as_str).unwrap_or(context)) else { continue };
        if let Some(p) = props.get(asset_id) {
            item.float = property(p, 2).and_then(|x| fraction(x.get("float_value"))).filter(|f| (0.0..=1.0).contains(f));
            item.pattern = property(p, 1).and_then(|x| whole(x.get("int_value"))).map(|n| n.min(u32::MAX as u64) as u32);
        }
        out.push(item);
    }
    let more = yes(v.get("more_items"));
    (more, v.get("last_assetid").and_then(Value::as_str).map(str::to_string))
}

/// Adds an inventory in the older form the trade offer window is given (one
/// map of assets, one of descriptions by "class_instance") to `out`.
pub fn parse_old_inventory(v: &Value, context: &str, out: &mut Vec<Cs2Item>) {
    let descriptions = v.get("rgDescriptions").and_then(Value::as_object);
    let Some(assets) = v.get("rgInventory").and_then(Value::as_object) else { return };
    let mut rows: Vec<(u64, Cs2Item)> = assets
        .values()
        .filter_map(|a| {
            let key = format!("{}_{}", a.get("classid")?.as_str()?, a.get("instanceid").and_then(Value::as_str).unwrap_or("0"));
            let d = descriptions?.get(&key)?;
            let item = item_from(d, a.get("id")?.as_str()?, context)?;
            Some((whole(a.get("pos")).unwrap_or(u64::MAX), item))
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.asset_id.cmp(&b.1.asset_id)));
    out.extend(rows.into_iter().map(|(_, item)| item));
}

async fn inventory_of(ws: &WebSession, steam_id: &str, context: &str, out: &mut Vec<Cs2Item>) -> Result<()> {
    let mut start: Option<String> = None;
    for _ in 0..20 {
        let mut url = community_url(&format!("/inventory/{steam_id}/{APP}/{context}?l=english&count=2000"));
        if let Some(s) = &start {
            url.push_str(&format!("&start_assetid={s}"));
        }
        let text = ws.get_text(&url).await?;
        let v: Value = serde_json::from_str(&text).context("inventory JSON")?;
        if v.is_null() {
            // Nothing in this inventory, or one that can't be seen.
            return Ok(());
        }
        let (more, last) = parse_inventory_page(&v, context, out);
        if !more || last.is_none() {
            break;
        }
        start = last;
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    Ok(())
}

/// The account's CS2 items: the ones it can sell and trade, and the ones
/// still trade protected.
pub async fn inventory(ws: &WebSession) -> Result<Vec<Cs2Item>> {
    let mut items = Vec::new();
    inventory_of(ws, &ws.steam_id, CONTEXT, &mut items).await?;
    // The protected ones are a second inventory; without it the first still counts.
    if let Err(e) = inventory_of(ws, &ws.steam_id, CONTEXT_PROTECTED, &mut items).await {
        if community::session_ended(&e) {
            return Err(e);
        }
        crate::logbus::debug("cs2", format!("The trade-protected items couldn't be read ({e:#}); showing the others"));
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!("fixtures/cs2_search_cases.json");
    const RIFLES: &str = include_str!("fixtures/cs2_search_rifles.json");
    const LISTINGS: &str = include_str!("fixtures/cs2_listings.json");
    const BOOK: &str = include_str!("fixtures/cs2_orderbook.json");
    const HISTORY: &str = include_str!("fixtures/cs2_price_history.json");
    const FACETS: &str = include_str!("fixtures/cs2_facets.json");

    #[test]
    fn steams_search_answer_for_cases_reads_as_items_sold_by_the_piece() {
        let hits = parse_hits(CASES).unwrap().unwrap();
        assert_eq!((hits.total, hits.start, hits.results.len()), (446, 0, 4));
        let first = &hits.results[0];
        assert_eq!((first.name.as_str(), first.hash_name.as_str(), first.kind.as_str()), ("Kilowatt Case", "Kilowatt Case", "Base Grade Container"));
        assert!(first.commodity && !first.grouped);
        // A case is asked for by its group's id, as Steam's own page does.
        assert_eq!(first.id, "G18A8263004");
        assert_eq!((first.listings, first.currency, first.price, first.price_text.as_str()), (670_581, 1, Some(9), "$0.09"));
        assert!(first.icon_url.starts_with("https://community.fastly.steamstatic.com/economy/image/i0CoZ81Ui0m"));
        assert_eq!(hits.results[2].name, "Dreams & Nightmares Case");
    }

    #[test]
    fn a_skin_in_steams_search_answer_stands_for_its_whole_group() {
        let hits = parse_hits(RIFLES).unwrap().unwrap();
        let ice = &hits.results[0];
        // One result for every wear of the skin: named after the group, with its cheapest kind's price.
        assert_eq!((ice.name.as_str(), ice.hash_name.as_str()), ("AK-47 | Ice Coaled", "AK-47 | Ice Coaled (Battle-Scarred)"));
        assert_eq!((ice.id.as_str(), ice.grouped, ice.commodity), ("G180720F7083004", true, false));
        assert_eq!((ice.price, ice.listings, ice.color.as_str()), (Some(442), 5196, "d32ce6"));
    }

    #[test]
    fn no_answer_from_the_search_is_told_apart_from_no_results() {
        assert_eq!(parse_hits(r#"{"data":null}"#).unwrap(), None);
        let none = parse_hits(r#"{"data":{"start":0,"total_count":0,"grouping":1,"facets":[],"results":[]}}"#).unwrap().unwrap();
        assert_eq!((none.total, none.results.len()), (0, 0));
        assert!(parse_hits("<html>Sorry!</html>").is_err());
        assert!(parse_hits(r#"{"data":{"oops":1}}"#).is_err());
    }

    #[test]
    fn a_search_is_asked_the_way_steams_page_asks() {
        let mut q = Query { text: "  redline ".into(), sort: 3, direction: 1, count: 500, price_min: Some(500), price_max: Some(0), ..Default::default() };
        q.filters.insert("Type".into(), vec!["CSGO_Type_Rifle".into()]);
        q.filters.insert("Exterior".into(), vec![]);
        q.filters.insert("bad name!".into(), vec!["x".into()]);
        q.filters.insert("Weapon".into(), vec!["weapon_ak47".into(), "\"},{".into()]);
        let (name, ask) = q.ask(31);
        assert_eq!(name, "Search");
        assert_eq!(
            ask,
            json!({"appid": 730, "strQuery": "redline", "filters": {"Type": ["CSGO_Type_Rifle"], "Weapon": ["weapon_ak47"]}, "accessoryFilters": {},
                   "price": {"eCurrency": 31, "unMin": 500}, "start": 0, "count": 100, "sort": 3, "direction": 1})
        );
        // Nothing picked: trending first, as Steam's page starts out.
        let (_, plain) = Query { count: 50, ..Default::default() }.ask(1);
        assert_eq!((plain["sort"].as_u64(), plain["direction"].as_u64(), plain.get("strQuery")), (Some(5), Some(2), None));
        // Descriptions are searched by Steam's other search, and only with words to look for.
        assert_eq!(Query { text: "phoenix".into(), descriptions: true, count: 10, ..Default::default() }.ask(1).0, "SearchDescriptions");
        assert_eq!(Query { descriptions: true, count: 10, ..Default::default() }.ask(1).0, "Search");
    }

    #[test]
    fn steams_listings_answer_reads_with_wear_pattern_stickers_and_fees() {
        let page = parse_item(LISTINGS).unwrap().unwrap();
        assert_eq!((page.total, page.start, page.more, page.listings.len()), (1705, 0, true, 2));
        let l = &page.listings[0];
        assert_eq!((l.listing_id.as_str(), l.asset_id.as_str()), ("5934539222390898406", "52797040720"));
        assert_eq!((l.hash_name.as_str(), l.variant.as_str()), ("AK-47 | Redline (Well-Worn)", "Well-Worn"));
        // What the seller gets, the fees on top, and what Steam writes out for the buyer.
        assert_eq!((l.price, l.fee, l.total, l.currency, l.price_text.as_str()), (7485, 1122, 8607, 31, "SAR 86.07"));
        assert_eq!(l.pattern, Some(67));
        assert!((l.float.unwrap() - 0.446_580_32).abs() < 1e-6);
        assert_eq!(l.inspect.as_deref(), Some("steam://run/730//+csgo_econ_action_preview%201909C9C9D2CEDD18011E39831B311C291D21B4D58BEE1A595A7B161118098B2D24A95208A45C1972DEA3715E69113FBD0B69"));
        assert_eq!(l.stickers.len(), 1);
        assert_eq!((l.stickers[0].name.as_str(), l.stickers[0].kind.as_str(), l.stickers[0].wear), ("Sticker | Team Liquid (Holo) | Paris 2023", "Exotic Sticker", Some(0.0)));
        // The copy's picture address was cut short: not passed on as a picture.
        assert_eq!(l.image, None);
        assert!(!l.mine);
        assert!(page.listings[1].stickers.is_empty());
        // The kinds the group comes in, with how many listings each has.
        let ft = page.facets.iter().find(|f| f.tag == "WearCategory2").unwrap();
        assert_eq!((ft.facet.as_str(), ft.facet_name.as_str(), ft.tag_name.as_str(), ft.listings), ("Exterior", "Exterior", "Field-Tested", 1395));
        let st = page.facets.iter().find(|f| f.tag == "strange").unwrap();
        assert_eq!((st.tag_name.as_str(), st.color.as_str(), st.listings), ("StatTrak™", "CF6A32", 224));
    }

    #[test]
    fn nothing_from_the_listings_lookup_means_ask_again() {
        assert_eq!(parse_item(r#"{"data":null}"#).unwrap(), None);
        // A case has no listings of its own: it sells from its order book.
        let case = parse_item(r#"{"data":{"more":true,"start":0,"total_count":1,"listings":[],"facets":[]}}"#).unwrap().unwrap();
        assert_eq!((case.total, case.listings.len()), (1, 0));
        assert!(parse_item("<html>").is_err());
    }

    #[test]
    fn an_order_book_reads_with_its_price_levels() {
        let d = parse_depth(BOOK).unwrap().unwrap();
        assert_eq!((d.currency, d.lowest_sell, d.highest_buy, d.sell_count, d.buy_count), (31, Some(9829), Some(9160), 1246, 47284));
        assert_eq!(d.sells[..3], [Level { price: 9829, quantity: 1 }, Level { price: 10225, quantity: 1 }, Level { price: 10450, quantity: 8 }]);
        assert_eq!(d.buys[..2], [Level { price: 9160, quantity: 2 }, Level { price: 9157, quantity: 1 }]);
        assert_eq!((d.sells.len(), d.buys.len(), d.more_sells, d.more_buys), (6, 6, 0, 0));
        // Steam sends every level there is, hundreds of them: only the first are kept.
        let many: Vec<u32> = (0..500).flat_map(|i| [1000 + i, 2]).collect();
        let big = json!({"data": {"success": true, "data": {"amtMinSellOrder": 1000, "amtMaxBuyOrder": 990, "eCurrency": 1, "cSellOrders": 1000, "cBuyOrders": 7, "rgCompactSellOrders": many, "rgCompactBuyOrders": []}}});
        let d = parse_depth(&big.to_string()).unwrap().unwrap();
        assert_eq!((d.sells.len(), d.more_sells, d.buys.len()), (LEVELS, 500 - LEVELS, 0));
        assert_eq!(parse_depth(r#"{"data":{"success":false}}"#).unwrap(), None);
    }

    #[test]
    fn a_price_history_reads_in_minor_units_oldest_first() {
        let h = parse_history(HISTORY).unwrap().unwrap();
        assert_eq!(h.currency, 31);
        assert_eq!(h.points.len(), 5);
        assert_eq!(h.points[0], (1_392_940_800, 15545, 198));
        assert_eq!(h.points[4], (1_795_798_800, 9831, 7));
        assert_eq!(parse_history(r#"{"data":null}"#).unwrap(), None);
        // Nothing sold yet: an empty history, not an error.
        assert_eq!(parse_history(r#"{"data":{"ecurrency":1,"prices":[]}}"#).unwrap().unwrap().points.len(), 0);
    }

    #[test]
    fn steams_filter_list_reads_with_names_counts_and_colors() {
        let facets = parse_facets(FACETS).unwrap();
        assert_eq!(facets.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["Exterior", "Quality", "Type"]);
        let quality = &facets[1];
        // Steam calls the filter "Category" and its tags by their own names.
        assert_eq!(quality.name, "Category");
        let st = quality.tags.iter().find(|t| t.id == "strange").unwrap();
        assert_eq!((st.name.as_str(), st.count, st.color.as_str()), ("StatTrak™", 290_588, "CF6A32"));
        assert_eq!(facets[0].tags.len(), 6);
        assert!(parse_facets(r#"{"success":true}"#).is_err());
    }

    /// As Steam's purchase dialog reads it (the account's own answer can't
    /// be had without signing in): written after that code, not copied.
    const BUYER: &str = r#"{"success":true,"country_code":"SA","require_billing_info":false,"confirmation_type":2,
        "wallet_info":{"wallet_currency":31,"wallet_balance":"12550","wallet_fee_percent":"0.05"},
        "tax_rate":{"tradefee_addtax":false,"tradefee_taxrate":"0","tax_region":""},
        "billing_address":{"firstname":"Given","lastname":"Family","address1":"1 Some Street","address2":"","city":"Town","state":"","postcode":"12345"}}"#;

    #[test]
    fn the_wallet_reads_and_the_address_stays_out_of_what_the_window_gets() {
        let b = parse_buyer(BUYER).unwrap();
        assert_eq!((b.currency, b.balance, b.country.as_str(), b.needs_billing, b.has_billing, b.confirmation), (31, Some(12550), "SA", false, true, 2));
        assert_eq!(b.tax_percent, 0.0);
        let for_window = serde_json::to_string(&b).unwrap();
        assert!(!for_window.contains("Given") && !for_window.contains("Some Street") && !for_window.contains("12345"), "{for_window}");
        assert!(!format!("{b:?}").contains("Given"));
        // What a visitor who isn't signed in gets: the session has ended.
        let e = parse_buyer(r#"{"success":false}"#).unwrap_err();
        assert!(community::session_ended(&e));
    }

    #[test]
    fn tax_on_fees_is_worked_out_as_steams_dialog_does() {
        let mut b = parse_buyer(BUYER).unwrap();
        assert_eq!(b.tax_on(1122), 0);
        b.tax_percent = 10.0;
        assert_eq!((b.tax_on(1122), b.tax_on(2), b.tax_on(3), b.tax_on(25)), (112, 0, 0, 3));
    }

    #[test]
    fn an_address_goes_along_only_where_steam_asks_for_one() {
        let mut b = parse_buyer(BUYER).unwrap();
        let sent = |b: &Buyer| billing_form(b, "0").into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
        assert_eq!(sent(&b), "first_name=&last_name=&billing_address=&billing_address_two=&billing_country=&billing_city=&billing_state=&billing_postal_code=&confirmation=0&save_my_address=0");
        b.needs_billing = true;
        assert!(sent(&b).starts_with("first_name=Given&last_name=Family&billing_address=1 Some Street&billing_address_two=&billing_country=SA&billing_city=Town"));
    }

    #[test]
    fn answers_to_a_purchase_are_told_apart() {
        // Bought: the wallet as it is afterwards.
        let ok = parse_bought(200, r#"{"wallet_info":{"wallet_currency":31,"wallet_balance":"3943"}}"#);
        assert_eq!((ok.ok, ok.needs_confirmation, ok.balance), (true, false, Some(3943)));
        // To confirm in the mobile app first.
        let wait = parse_bought(406, r#"{"need_confirmation":true,"confirmation":{"confirmation_id":"8817263544"}}"#);
        assert_eq!((wait.ok, wait.needs_confirmation, wait.confirmation_id.as_deref()), (false, true, Some("8817263544")));
        // Refused, with Steam's reason.
        let gone = parse_bought(502, r#"{"message":"The listing you tried to purchase is no longer available."}"#);
        assert_eq!((gone.ok, gone.needs_confirmation, gone.message.as_str()), (false, false, "The listing you tried to purchase is no longer available."));
        // An answer that isn't Steam's: refused, with the status.
        assert_eq!(parse_bought(500, "<html>").message, "Steam refused (HTTP 500)");
        assert!(!parse_bought(200, "<html>").ok);
        // A buy order placed, and one Steam says no to with a 200.
        let order = parse_bought(200, r#"{"success":1,"buy_orderid":"7340298113"}"#);
        assert_eq!((order.ok, order.order_id.as_deref()), (true, Some("7340298113")));
        let no = parse_bought(200, r#"{"success":25,"message":"You already have an active buy order for this item."}"#);
        assert_eq!((no.ok, no.message.as_str()), (false, "You already have an active buy order for this item."));
    }

    #[test]
    fn a_buy_orders_status_reads() {
        let open = parse_order_status(r#"{"success":1,"active":1,"purchased":0,"quantity":"3","quantity_remaining":"3","purchases":[]}"#).unwrap();
        assert_eq!((open.active, open.quantity, open.remaining, open.bought), (true, 3, 3, 0));
        let filled = parse_order_status(r#"{"success":1,"active":0,"purchased":1,"quantity":"1","quantity_remaining":"0","purchases":[{"appid":730,"contextid":"2","assetid":"1"}],"purchase_amount_text":"SAR 3.02"}"#).unwrap();
        assert_eq!((filled.active, filled.bought, filled.paid_text.as_str()), (false, 1, "SAR 3.02"));
        assert!(parse_order_status(r#"{"success":false}"#).is_err());
    }

    #[test]
    fn own_listings_and_orders_read_and_only_the_cs2_ones() {
        let text = r#"{"success":true,"total_count":2,
          "listings":[{"listingid":"111","time_created":1795000000,"price":870,"fee":130,"currencyid":"2031",
             "asset":{"appid":730,"contextid":"2","id":"9","market_hash_name":"Fracture Case","market_name":"Fracture Case","name_color":"b0c3d9","icon_url":"abc"}},
            {"listingid":"222","price":3,"fee":2,"currencyid":"2031","asset":{"appid":753,"market_hash_name":"440-Scout"}}],
          "listings_to_confirm":[{"listingid":"333","price":7485,"fee":1122,"currencyid":"2031","asset":{"appid":730,"market_hash_name":"AK-47 | Redline (Well-Worn)","name":"AK-47 | Redline"}}],
          "listings_on_hold":[],
          "buy_orders":[{"appid":730,"hash_name":"Kilowatt Case","wallet_currency":2031,"price":"30","quantity":"5","quantity_remaining":"4","buy_orderid":"444","description":{"market_name":"Kilowatt Case","icon_url":"def"}},
                        {"appid":753,"hash_name":"x","buy_orderid":"555"}]}"#;
        let mine = parse_mine(text).unwrap();
        assert_eq!(mine.listings.iter().map(|l| (l.listing_id.as_str(), l.status.as_str())).collect::<Vec<_>>(), [("111", "active"), ("333", "pending")]);
        assert_eq!((mine.listings[0].buyer_pays, mine.listings[0].you_receive, mine.listings[0].currency), (1000, 870, 31));
        assert_eq!(mine.listings[1].name, "AK-47 | Redline");
        assert_eq!(mine.orders.len(), 1);
        let o = &mine.orders[0];
        assert_eq!((o.order_id.as_str(), o.price, o.quantity, o.remaining, o.currency), ("444", 30, 5, 4, 31));
        assert!(community::session_ended(&parse_mine(r#"{"success":false}"#).unwrap_err()));
    }

    #[test]
    fn a_cs2_inventory_reads_with_wear_and_what_can_be_done_with_each_item() {
        let v = json!({
            "assets": [
                {"appid": 730, "contextid": "2", "assetid": "100", "classid": "7", "instanceid": "1", "amount": "1"},
                {"appid": 730, "contextid": "2", "assetid": "101", "classid": "8", "instanceid": "0", "amount": "1"},
                {"appid": 730, "contextid": "2", "assetid": "102", "classid": "404", "instanceid": "0", "amount": "1"}
            ],
            "descriptions": [
                {"classid": "7", "instanceid": "1", "name": "AK-47 | Redline", "market_name": "AK-47 | Redline (Field-Tested)", "market_hash_name": "AK-47 | Redline (Field-Tested)",
                 "type": "Classified Rifle", "name_color": "d32ce6", "icon_url": "ic", "tradable": 1, "marketable": 1, "commodity": 0,
                 "tags": [{"category": "Exterior", "internal_name": "WearCategory2", "localized_tag_name": "Field-Tested"}]},
                {"classid": "8", "instanceid": "0", "name": "Fracture Case", "market_hash_name": "Fracture Case", "type": "Base Grade Container", "tradable": 0, "marketable": 0, "commodity": 1,
                 "owner_descriptions": [{"type": "html", "value": " "}, {"type": "html", "value": "<span>Tradable/Marketable After Dec 04, 2026 (8:00:00) GMT</span>"}]}
            ],
            "asset_properties": [{"appid": 730, "contextid": "2", "assetid": "100", "asset_properties": [{"propertyid": 1, "int_value": "661"}, {"propertyid": 2, "float_value": "0.2231"}]}],
            "more_items": 1, "last_assetid": "101", "total_inventory_count": 2
        });
        let mut items = Vec::new();
        let (more, last) = parse_inventory_page(&v, CONTEXT, &mut items);
        assert_eq!((more, last.as_deref()), (true, Some("101")));
        // The asset without a description is left out, not guessed at.
        assert_eq!(items.len(), 2);
        let ak = &items[0];
        assert_eq!((ak.asset_id.as_str(), ak.exterior.as_str(), ak.pattern, ak.marketable, ak.tradable, ak.protected), ("100", "Field-Tested", Some(661), true, true, false));
        assert!((ak.float.unwrap() - 0.2231).abs() < 1e-9);
        let case = &items[1];
        assert_eq!((case.marketable, case.commodity, case.notes.as_slice()), (false, true, ["Tradable/Marketable After Dec 04, 2026 (8:00:00) GMT".to_string()].as_slice()));
        // The same item in the second inventory: there, but not to sell or trade yet.
        let mut held = Vec::new();
        let one = json!({"assets": [{"assetid": "100", "classid": "7", "instanceid": "1", "contextid": "16"}], "descriptions": v["descriptions"]});
        parse_inventory_page(&one, CONTEXT_PROTECTED, &mut held);
        assert_eq!((held[0].context.as_str(), held[0].protected, held[0].marketable, held[0].tradable), ("16", true, false, false));
    }

    #[test]
    fn the_trade_windows_older_inventory_form_reads_in_its_own_order() {
        let v = json!({"success": true,
            "rgInventory": {"31": {"id": "31", "classid": "8", "instanceid": "0", "amount": "1", "pos": 2}, "30": {"id": "30", "classid": "7", "instanceid": "1", "amount": "1", "pos": 1}},
            "rgDescriptions": {"7_1": {"classid": "7", "instanceid": "1", "name": "AK-47 | Redline", "market_hash_name": "AK-47 | Redline (Field-Tested)", "tradable": 1, "marketable": 1},
                               "8_0": {"classid": "8", "instanceid": "0", "name": "Fracture Case", "market_hash_name": "Fracture Case", "tradable": 1, "commodity": 1}},
            "more": false});
        let mut items = Vec::new();
        parse_old_inventory(&v, CONTEXT, &mut items);
        assert_eq!(items.iter().map(|i| (i.asset_id.as_str(), i.name.as_str(), i.tradable)).collect::<Vec<_>>(), [("30", "AK-47 | Redline", true), ("31", "Fracture Case", true)]);
    }

    #[test]
    fn suggestions_for_the_search_box_read() {
        let text = r#"{"results":[{"market_name":"AK-47 | Asiimov","market_hash_name":"G180720A1063004","app_id":730,"app_name":"Counter-Strike 2","icon_url":"https://x/y.png"},
                                   {"market_name":"Other game's item","market_hash_name":"x","app_id":440}],"apps":[]}"#;
        let s = parse_suggestions(text);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].id.as_str(), s[0].name.as_str()), ("G180720A1063004", "AK-47 | Asiimov"));
        assert!(parse_suggestions("<html>").is_empty());
    }
}
