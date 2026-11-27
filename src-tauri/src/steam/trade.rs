//! Trade offers: the open ones (through Steam's Web API, with the web
//! session's own sign-in), another player's CS2 items, and sending,
//! accepting, declining and cancelling an offer the way Steam's trade offer
//! window does (`economy_tradeoffer.js`).
//!
//! None of this could be tried on Steam without an account's sign-in: the
//! requests follow Steam's page code and its Web API documentation, and the
//! answers are read tolerantly.
use super::community::{api_url, community_url, SessionEnded, SteamBusy, WebSession};
use super::cs2::{self, Cs2Item};
use crate::net::Logged;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// What a Steam ID is counted up from: an account's number plus this.
const STEAM_ID_BASE: u64 = 76_561_197_960_265_728;

/// Who an offer goes to: their account number, and the token from their
/// trade link, which lets someone who isn't a friend send them offers. The
/// token is theirs to give out, and is kept out of every log line.
#[derive(Clone, PartialEq)]
pub struct TradeLink {
    pub account: u32,
    pub token: String,
}

impl std::fmt::Debug for TradeLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TradeLink(account …{}, token {})", self.account % 10_000, if self.token.is_empty() { "none" } else { "given" })
    }
}

impl TradeLink {
    pub fn steam_id(&self) -> String {
        (STEAM_ID_BASE + self.account as u64).to_string()
    }

    /// The page Steam's own offer window is on, which every request names.
    fn page(&self) -> String {
        let token = if self.token.is_empty() { String::new() } else { format!("&token={}", self.token) };
        format!("https://steamcommunity.com/tradeoffer/new/?partner={}{token}", self.account)
    }
}

/// Reads who to trade with from what was pasted: a trade link
/// (`…/tradeoffer/new/?partner=…&token=…`), or a friend's 17-digit Steam ID.
pub fn parse_trade_link(input: &str) -> Result<TradeLink> {
    let input = input.trim();
    if input.len() == 17 && input.chars().all(|c| c.is_ascii_digit()) {
        let id: u64 = input.parse()?;
        let account = id.checked_sub(STEAM_ID_BASE).and_then(|a| u32::try_from(a).ok()).filter(|&a| a > 0);
        return account.map(|account| TradeLink { account, token: String::new() }).ok_or_else(|| anyhow!("That isn't a player's Steam ID"));
    }
    let url = reqwest::Url::parse(input).map_err(|_| anyhow!("Paste the other player's trade link (it starts with https://steamcommunity.com/tradeoffer/new/)"))?;
    if url.host_str() != Some("steamcommunity.com") || !url.path().trim_end_matches('/').eq_ignore_ascii_case("/tradeoffer/new") {
        bail!("That isn't a trade link: it should start with https://steamcommunity.com/tradeoffer/new/");
    }
    let field = |name: &str| url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned());
    let account = field("partner").and_then(|p| p.parse::<u32>().ok()).filter(|&a| a > 0).ok_or_else(|| anyhow!("The trade link doesn't say whose it is (no partner number)"))?;
    let token = field("token").unwrap_or_default();
    if token.len() > 32 || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        bail!("The trade link's token doesn't look right");
    }
    Ok(TradeLink { account, token })
}

// ---------------------------------------------------------------- offers ---

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TradeItem {
    pub app: u32,
    pub context: String,
    pub asset_id: String,
    pub name: String,
    pub kind: String,
    pub icon_url: String,
    pub color: String,
    /// No longer in the inventory it was offered from.
    pub missing: bool,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub id: String,
    /// The other player's Steam ID.
    pub partner: String,
    pub message: String,
    /// Steam's state number: 2 active, 9 waiting for the sender's confirmation, 11 on hold.
    pub state: u32,
    pub state_text: String,
    /// Sent by this account.
    pub ours: bool,
    pub created: i64,
    pub updated: i64,
    pub expires: i64,
    /// What this account gives, and what it gets.
    pub give: Vec<TradeItem>,
    pub receive: Vec<TradeItem>,
    /// When a held trade is delivered; 0 when it isn't held.
    pub held_until: i64,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Offers {
    pub received: Vec<Offer>,
    pub sent: Vec<Offer>,
}

fn state_text(state: u32) -> &'static str {
    match state {
        2 => "Active",
        3 => "Accepted",
        4 => "Countered",
        5 => "Expired",
        6 => "Cancelled",
        7 => "Declined",
        8 => "Items no longer available",
        9 => "Waiting to be confirmed",
        10 => "Cancelled by its sender's second factor",
        11 => "On hold",
        _ => "Unknown",
    }
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn number(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Reads Steam's list of trade offers with the descriptions of their items.
pub fn parse_offers(text_: &str) -> Result<Offers> {
    let v: Value = serde_json::from_str(text_).map_err(|_| anyhow!("Steam answered with a page instead of the trade offers"))?;
    let r = v.get("response").filter(|r| r.is_object()).ok_or_else(|| anyhow!("Steam's trade offers answer isn't in the expected form"))?;
    let mut described: std::collections::HashMap<(u64, String, String), &Value> = std::collections::HashMap::new();
    for d in r.get("descriptions").and_then(Value::as_array).into_iter().flatten() {
        described.insert((number(d.get("appid")), text(d, "classid"), Some(text(d, "instanceid")).filter(|i| !i.is_empty()).unwrap_or_else(|| "0".into())), d);
    }
    let items = |list: Option<&Value>| -> Vec<TradeItem> {
        list.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|a| {
                let app = number(a.get("appid"));
                let key = (app, text(a, "classid"), Some(text(a, "instanceid")).filter(|i| !i.is_empty()).unwrap_or_else(|| "0".into()));
                let d = described.get(&key).copied().unwrap_or(&Value::Null);
                let name = Some(text(d, "market_name")).filter(|n| !n.is_empty()).unwrap_or_else(|| text(d, "name"));
                TradeItem {
                    app: app as u32,
                    context: text(a, "contextid"),
                    asset_id: text(a, "assetid"),
                    name: if name.is_empty() { format!("Item of app {app}") } else { name },
                    kind: text(d, "type"),
                    icon_url: cs2::icon(&text(d, "icon_url"), 96),
                    color: text(d, "name_color"),
                    missing: a.get("missing").and_then(Value::as_bool).unwrap_or(false),
                }
            })
            .collect()
    };
    let offers = |key: &str| -> Vec<Offer> {
        r.get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| {
                let id = text(o, "tradeofferid");
                let account = number(o.get("accountid_other"));
                if id.is_empty() || account == 0 {
                    return None;
                }
                let state = number(o.get("trade_offer_state")) as u32;
                Some(Offer {
                    id,
                    partner: (STEAM_ID_BASE + account).to_string(),
                    message: text(o, "message"),
                    state,
                    state_text: state_text(state).to_string(),
                    ours: o.get("is_our_offer").and_then(Value::as_bool).unwrap_or(false),
                    created: number(o.get("time_created")) as i64,
                    updated: number(o.get("time_updated")) as i64,
                    expires: number(o.get("expiration_time")) as i64,
                    give: items(o.get("items_to_give")),
                    receive: items(o.get("items_to_receive")),
                    held_until: number(o.get("escrow_end_date")) as i64,
                })
            })
            .collect()
    };
    let newest_first = |mut list: Vec<Offer>| {
        list.sort_by(|a, b| b.updated.max(b.created).cmp(&a.updated.max(a.created)).then(b.id.cmp(&a.id)));
        list
    };
    Ok(Offers { received: newest_first(offers("trade_offers_received")), sent: newest_first(offers("trade_offers_sent")) })
}

/// The open trade offers, received and sent. Steam's Web API takes the web
/// session's sign-in in place of a key.
pub async fn offers(ws: &WebSession, now: i64) -> Result<Offers> {
    if ws.access_token.is_empty() {
        bail!("This web session has no sign-in the trade offers can be read with; sign in again from Accounts");
    }
    let url = api_url("/IEconService/GetTradeOffers/v1/");
    let resp = ws
        .client
        .get(url)
        .query(&[
            ("access_token", ws.access_token.as_str()),
            ("get_sent_offers", "1"),
            ("get_received_offers", "1"),
            ("get_descriptions", "1"),
            ("language", "english"),
            ("active_only", "1"),
            ("time_historical_cutoff", &now.to_string()),
        ])
        .logged()
        .await
        .map_err(|e| anyhow::Error::new(e.without_url()))
        .context("GET the trade offers")?;
    let status = resp.status().as_u16();
    match status {
        401 | 403 => return Err(SessionEnded.into()),
        429 => return Err(SteamBusy(status).into()),
        s if s >= 500 => return Err(SteamBusy(s).into()),
        s if !(200..300).contains(&s) => bail!("Steam's Web API answered HTTP {s} for the trade offers"),
        _ => {}
    }
    let text = resp.text().await.map_err(|e| anyhow::Error::new(e.without_url())).context("GET the trade offers")?;
    parse_offers(&text)
}

/// The other player's CS2 items that can be traded, as Steam shows them in
/// the offer window. When Steam won't show them that way, their inventory
/// is read as anyone can see it, if it is public.
pub async fn partner_inventory(ws: &WebSession, link: &TradeLink) -> Result<Vec<Cs2Item>> {
    let partner = link.steam_id();
    let url = community_url(&format!(
        "/tradeoffer/new/partnerinventory/?sessionid={}&partner={partner}&appid={}&contextid={}",
        ws.session_id,
        cs2::APP,
        cs2::CONTEXT
    ));
    let resp = ws.client.get(url).header("Referer", link.page()).logged().await.map_err(|e| anyhow::Error::new(e.without_url())).context("GET the other player's items")?;
    let status = resp.status().as_u16();
    if status == 429 {
        return Err(SteamBusy(status).into());
    }
    let body = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let mut items = Vec::new();
    if (200..300).contains(&status) && v.is_object() {
        if v.get("rgInventory").is_some() {
            cs2::parse_old_inventory(&v, cs2::CONTEXT, &mut items);
        } else {
            cs2::parse_inventory_page(&v, cs2::CONTEXT, &mut items);
        }
        if v.get("success").is_none_or(|s| s.as_bool() != Some(false)) {
            return Ok(items.into_iter().filter(|i| i.tradable).collect());
        }
    }
    crate::logbus::debug("trade", format!("Steam didn't show the other player's items for an offer (HTTP {status}); reading their inventory as anyone sees it"));
    let text = ws.get_text(&community_url(&format!("/inventory/{partner}/{}/{}?l=english&count=2000", cs2::APP, cs2::CONTEXT))).await?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if v.is_null() {
        bail!("Steam shows none of the other player's CS2 items: their inventory is private, or the trade link is no longer good");
    }
    cs2::parse_inventory_page(&v, cs2::CONTEXT, &mut items);
    Ok(items.into_iter().filter(|i| i.tradable).collect())
}

/// One item of an offer, named by where it is.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssetRef {
    pub app: u32,
    pub context: String,
    pub asset_id: String,
}

/// An offer as Steam's offer window writes it down: what "me" gives and
/// what "them" gives.
pub fn offer_json(give: &[AssetRef], receive: &[AssetRef]) -> Value {
    let side = |assets: &[AssetRef]| {
        json!({
            "assets": assets.iter().map(|a| json!({"appid": a.app, "contextid": a.context, "amount": 1, "assetid": a.asset_id})).collect::<Vec<_>>(),
            "currency": [],
            "ready": false,
        })
    };
    json!({ "newversion": true, "version": give.len() + receive.len() + 1, "me": side(give), "them": side(receive) })
}

/// How sending or accepting an offer ended.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TradeOutcome {
    pub ok: bool,
    pub offer_id: Option<String>,
    /// Still to be confirmed in the Steam mobile app, or by e-mail.
    pub needs_confirmation: bool,
    pub by_email: bool,
    pub message: String,
}

/// What Steam's numbered refusals most often mean.
fn hint(error: &str) -> Option<&'static str> {
    let code: u32 = re!(r"\((\d+)\)\s*$").captures(error.trim())?.get(1)?.as_str().parse().ok()?;
    Some(match code {
        15 => "the other player can't trade right now, their inventory is full, or the trade link is no longer good",
        16 => "Steam took too long: the offer may have been sent all the same, so look at the sent offers before trying again",
        20 => "Steam's trading is down for the moment",
        25 => "there are too many open offers from this account: cancel some first",
        26 => "one of the items is no longer where it was",
        28 => "the offer has already been answered",
        50 => "there are too many open offers to this player",
        _ => return None,
    })
}

/// Reads Steam's answer to sending or accepting an offer.
pub fn parse_outcome(status: u16, text_: &str) -> TradeOutcome {
    let v: Value = serde_json::from_str(text_).unwrap_or(Value::Null);
    let error = text(&v, "strError");
    let good = (200..300).contains(&status) && v.is_object() && error.is_empty();
    if !good {
        let said = if error.is_empty() { format!("Steam refused (HTTP {status})") } else { error.split_whitespace().collect::<Vec<_>>().join(" ") };
        let message = match hint(&said) {
            Some(more) => format!("{said} Usually this means {more}."),
            None => said,
        };
        return TradeOutcome { message, ..Default::default() };
    }
    let by_email = v.get("needs_email_confirmation").and_then(Value::as_bool).unwrap_or(false);
    let needs_confirmation = by_email || v.get("needs_mobile_confirmation").and_then(Value::as_bool).unwrap_or(false);
    let offer_id = Some(text(&v, "tradeofferid")).filter(|id| !id.is_empty());
    // Sending names the new offer; accepting names the trade, or asks to confirm.
    let done = offer_id.is_some() || v.get("tradeid").is_some() || needs_confirmation;
    TradeOutcome {
        ok: done,
        offer_id,
        needs_confirmation,
        by_email,
        message: if done { String::new() } else { "Steam's answer didn't say whether it worked: look at the offer on Steam before trying again".into() },
    }
}

fn digits(id: &str) -> Result<&str> {
    if id.is_empty() || id.len() > 20 || !id.chars().all(|c| c.is_ascii_digit()) {
        bail!("That isn't a trade offer's number");
    }
    Ok(id)
}

/// Sends a trade offer. Sent once, never again by itself: an offer that
/// gives items away mustn't go out twice.
pub async fn send(ws: &WebSession, link: &TradeLink, give: &[AssetRef], receive: &[AssetRef], message: &str) -> Result<TradeOutcome> {
    if give.is_empty() && receive.is_empty() {
        bail!("An offer needs at least one item");
    }
    let offer = offer_json(give, receive).to_string();
    let params = if link.token.is_empty() { json!({}) } else { json!({ "trade_offer_access_token": link.token }) }.to_string();
    let partner = link.steam_id();
    let message: String = message.trim().chars().take(128).collect();
    let form = [
        ("sessionid", ws.session_id.as_str()),
        ("serverid", "1"),
        ("partner", partner.as_str()),
        ("tradeoffermessage", message.as_str()),
        ("json_tradeoffer", offer.as_str()),
        ("captcha", ""),
        ("trade_offer_create_params", params.as_str()),
    ];
    let (status, text) = ws.post_form(&community_url("/tradeoffer/new/send"), &link.page(), &form).await?;
    Ok(parse_outcome(status, &text))
}

/// Accepts a received offer, as the offer's own page does.
pub async fn accept(ws: &WebSession, offer_id: &str, partner: &str) -> Result<TradeOutcome> {
    let id = digits(offer_id)?;
    let form = [("sessionid", ws.session_id.as_str()), ("serverid", "1"), ("tradeofferid", id), ("partner", partner), ("captcha", "")];
    let (status, text) = ws.post_form(&community_url(&format!("/tradeoffer/{id}/accept")), &format!("https://steamcommunity.com/tradeoffer/{id}/"), &form).await?;
    Ok(parse_outcome(status, &text))
}

/// Declines a received offer (`decline`) or takes back a sent one (`cancel`).
pub async fn withdraw(ws: &WebSession, offer_id: &str, how: &str) -> Result<()> {
    let id = digits(offer_id)?;
    if how != "decline" && how != "cancel" {
        bail!("An offer is declined or cancelled");
    }
    let form = [("sessionid", ws.session_id.as_str())];
    let referer = format!("https://steamcommunity.com/profiles/{}/tradeoffers/", ws.steam_id);
    let (status, text) = ws.post_form(&community_url(&format!("/tradeoffer/{id}/{how}")), &referer, &form).await?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let error = self::text(&v, "strError");
    if (200..300).contains(&status) && error.is_empty() && v.get("success").is_none_or(|s| s.as_bool() != Some(false)) {
        return Ok(());
    }
    let why = if error.is_empty() { format!("HTTP {status}") } else { error };
    bail!("Steam didn't {how} the offer: {why}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trade_link_reads_and_never_shows_its_token() {
        let link = parse_trade_link(" https://steamcommunity.com/tradeoffer/new/?partner=46143802&token=aB3_dE-9 ").unwrap();
        assert_eq!((link.account, link.token.as_str()), (46_143_802, "aB3_dE-9"));
        assert_eq!(link.steam_id(), "76561198006409530");
        assert_eq!(link.page(), "https://steamcommunity.com/tradeoffer/new/?partner=46143802&token=aB3_dE-9");
        let shown = format!("{link:?}");
        assert!(!shown.contains("aB3_dE-9") && !shown.contains("46143802"), "{shown}");
        // A friend needs no token: their Steam ID is enough.
        let friend = parse_trade_link("76561198006409530").unwrap();
        assert_eq!((friend.account, friend.token.as_str()), (46_143_802, ""));
        assert_eq!(friend.page(), "https://steamcommunity.com/tradeoffer/new/?partner=46143802");
    }

    #[test]
    fn what_isnt_a_trade_link_is_refused() {
        for bad in [
            "",
            "hello",
            "https://example.com/tradeoffer/new/?partner=1&token=a",
            "https://steamcommunity.com/tradeoffer/new/?token=abc",
            "https://steamcommunity.com/tradeoffer/new/?partner=0&token=abc",
            "https://steamcommunity.com/tradeoffer/new/?partner=12&token=a%22%2C%22x",
            "https://steamcommunity.com/id/someone/",
            "76561197960265728",
            "12345678901234567",
        ] {
            assert!(parse_trade_link(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_offer_is_written_as_steams_window_writes_it() {
        let give = [AssetRef { app: 730, context: "2".into(), asset_id: "100".into() }];
        let receive = [AssetRef { app: 730, context: "2".into(), asset_id: "200".into() }, AssetRef { app: 730, context: "2".into(), asset_id: "201".into() }];
        assert_eq!(
            offer_json(&give, &receive),
            json!({"newversion": true, "version": 4,
                   "me": {"assets": [{"appid": 730, "contextid": "2", "amount": 1, "assetid": "100"}], "currency": [], "ready": false},
                   "them": {"assets": [{"appid": 730, "contextid": "2", "amount": 1, "assetid": "200"}, {"appid": 730, "contextid": "2", "amount": 1, "assetid": "201"}], "currency": [], "ready": false}})
        );
    }

    /// In the form Steam's Web API documents for GetTradeOffers: written
    /// after the documentation, since the real answer needs a sign-in.
    const OFFERS: &str = r#"{"response":{
      "trade_offers_received":[
        {"tradeofferid":"6001","accountid_other":46143802,"message":"for the case","expiration_time":1797000000,"trade_offer_state":2,
         "items_to_give":[{"appid":730,"contextid":"2","assetid":"100","classid":"7","instanceid":"1","amount":"1","missing":false}],
         "items_to_receive":[{"appid":730,"contextid":"2","assetid":"900","classid":"8","instanceid":"0","amount":"1","missing":false}],
         "is_our_offer":false,"time_created":1795800000,"time_updated":1795800500,"from_real_time_trade":false,"escrow_end_date":0,"confirmation_method":0},
        {"tradeofferid":"6000","accountid_other":5,"message":"","trade_offer_state":2,"items_to_receive":[{"appid":440,"contextid":"2","assetid":"1","classid":"99","instanceid":"0","amount":"1"}],
         "is_our_offer":false,"time_created":1795700000,"time_updated":1795700000}],
      "trade_offers_sent":[
        {"tradeofferid":"6002","accountid_other":46143802,"message":"","trade_offer_state":9,"items_to_give":[{"appid":730,"contextid":"2","assetid":"101","classid":"8","instanceid":"0","amount":"1","missing":true}],
         "is_our_offer":true,"time_created":1795900000,"time_updated":1795900000,"escrow_end_date":1796500000}],
      "descriptions":[
        {"appid":730,"classid":"7","instanceid":"1","name":"AK-47 | Redline","market_name":"AK-47 | Redline (Field-Tested)","type":"Classified Rifle","name_color":"d32ce6","icon_url":"ak"},
        {"appid":730,"classid":"8","instanceid":"0","name":"Fracture Case","type":"Base Grade Container","icon_url":"case"}]}}"#;

    #[test]
    fn open_offers_read_with_what_each_side_gives() {
        let o = parse_offers(OFFERS).unwrap();
        assert_eq!(o.received.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["6001", "6000"]);
        let first = &o.received[0];
        assert_eq!((first.partner.as_str(), first.message.as_str(), first.state, first.state_text.as_str(), first.ours), ("76561198006409530", "for the case", 2, "Active", false));
        assert_eq!(first.give.iter().map(|i| (i.name.as_str(), i.asset_id.as_str())).collect::<Vec<_>>(), [("AK-47 | Redline (Field-Tested)", "100")]);
        assert_eq!(first.receive[0].name, "Fracture Case");
        assert!(first.receive[0].icon_url.ends_with("/case/96fx96f"));
        // An item Steam sent no description for still shows, by what is known of it.
        assert_eq!((o.received[1].receive[0].name.as_str(), o.received[1].give.len()), ("Item of app 440", 0));
        let sent = &o.sent[0];
        assert_eq!((sent.state_text.as_str(), sent.ours, sent.held_until, sent.give[0].missing), ("Waiting to be confirmed", true, 1_796_500_000, true));
        // No offers at all.
        let none = parse_offers(r#"{"response":{}}"#).unwrap();
        assert!(none.received.is_empty() && none.sent.is_empty());
        assert!(parse_offers("<html>").is_err());
        assert!(parse_offers("{}").is_err());
    }

    #[test]
    fn answers_to_sending_and_accepting_are_told_apart() {
        let sent = parse_outcome(200, r#"{"tradeofferid":"6003","needs_mobile_confirmation":true,"needs_email_confirmation":false,"email_domain":"example.com"}"#);
        assert_eq!((sent.ok, sent.offer_id.as_deref(), sent.needs_confirmation, sent.by_email), (true, Some("6003"), true, false));
        let accepted = parse_outcome(200, r#"{"tradeid":"771"}"#);
        assert_eq!((accepted.ok, accepted.needs_confirmation), (true, false));
        let refused = parse_outcome(500, r#"{"strError":"There was an error sending your trade offer.  Please try again later. (15)"}"#);
        assert!(!refused.ok);
        assert_eq!(
            refused.message,
            "There was an error sending your trade offer. Please try again later. (15) Usually this means the other player can't trade right now, their inventory is full, or the trade link is no longer good."
        );
        assert_eq!(parse_outcome(400, "").message, "Steam refused (HTTP 400)");
        // "OK" without a word about the offer: not counted as sent.
        assert!(!parse_outcome(200, "{}").ok);
        assert!(!parse_outcome(200, "<html>").ok);
    }
}
