//! Steam's upcoming sales and fests. Valve lists them on a public page of the
//! Steamworks documentation; the window asks for it once a day, after midnight,
//! and the list is kept on disk, so it shows at once and while the page can't
//! be read. Sites that show the same list, like SteamDB, ask apps not to read
//! them, so this goes to Valve's own page.

use crate::logbus;
use anyhow::{anyhow, bail, Result};
use chrono::{Datelike, NaiveDate, NaiveTime, Weekday};
use scraper::{ElementRef, Html, Node};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Valve's list of upcoming events, in English: other languages name the
/// months differently.
pub const SOURCE: &str = "https://partner.steamgames.com/doc/marketing/upcoming_events?l=english";

/// Where the last list read is kept, in the app's data folder.
pub const FILE: &str = "steam_events.json";

/// Where the page is read from. Development builds can be pointed at a local
/// copy instead, for trying this out without Valve's server.
fn source() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("IDLEKIT_EVENTS_URL") {
        return url;
    }
    SOURCE.to_string()
}

/// Valve's page as it was when this version was made: what is shown before
/// the page has ever been read.
const BUILT_IN: &str = include_str!("fixtures/upcoming_events.html");

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SteamEvent {
    /// As Valve names it ("Winter Sale 2026", "Racing Fest"); the Next Fests,
    /// which share a name, get their month ("Next Fest: February 2027").
    pub name: String,
    /// "sale" (the seasonal sales), "fest" (themed sale events), "nextfest" or "event".
    pub kind: String,
    /// The first and the last day as Valve gives them (YYYY-MM-DD). None while
    /// Valve says the dates are still to be decided.
    pub first_day: Option<String>,
    pub last_day: Option<String>,
    /// When it starts and ends, in ms since 1970. Steam's events change over
    /// at 10 AM Pacific time, on the first and on the last day.
    pub starts: Option<i64>,
    pub ends: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventList {
    pub events: Vec<SteamEvent>,
    /// When the list was read from Valve's page (ms). None for the list that
    /// comes with this version of IdleKit.
    pub fetched_at: Option<i64>,
    /// "valve" or "built-in".
    pub from: String,
    /// Why the page couldn't be read this time, when it couldn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Reads Valve's page. A page without a single date IdleKit can read counts
/// as a failure, so a changed page never replaces a good list with nothing.
pub async fn fetch(http: &reqwest::Client) -> Result<Vec<SteamEvent>> {
    let resp = http.get(source()).header("Accept-Language", "en-US,en;q=0.8").send().await.map_err(|e| {
        anyhow!(if e.is_timeout() { "Valve's page didn't answer in time" } else { "Valve's page couldn't be reached" })
    })?;
    if !resp.status().is_success() {
        bail!("Valve's page answered HTTP {}", resp.status().as_u16());
    }
    let html = resp.text().await.map_err(|_| anyhow!("Valve's page was cut off on the way"))?;
    let events = parse(&html);
    if !events.iter().any(|e| e.starts.is_some()) {
        bail!("Valve's page had no dates IdleKit could read");
    }
    Ok(events)
}

/// The list for the window. Without `refresh` it is what is there already,
/// the list read last or the one this version came with, and the network
/// isn't touched: the window asks for a fresh copy itself when that one is
/// from before today. With `refresh`, Valve's page is read with `read` and
/// kept; a page that can't be read leaves what was there, with the reason.
pub async fn list<F, Fut>(file: &Path, refresh: bool, read: F) -> EventList
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<SteamEvent>>>,
{
    let kept = load(file);
    if !refresh {
        return kept.unwrap_or_else(built_in);
    }
    // The same failure again (an hour later, offline) is no news: a warning
    // the first time, then only a debug line.
    static LAST_FAILURE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    match read().await {
        Ok(events) => {
            *LAST_FAILURE.lock().unwrap_or_else(|p| p.into_inner()) = None;
            let list =
                EventList { events, fetched_at: Some(chrono::Utc::now().timestamp_millis()), from: "valve".into(), error: None };
            logbus::info("events", format!("Read {} Steam events from Valve's schedule", list.events.len()));
            if let Err(e) = save(file, &list) {
                logbus::warn("events", format!("Couldn't keep the events list on disk: {e:#}"));
            }
            list
        }
        Err(e) => {
            let why = format!("{e:#}");
            let again = LAST_FAILURE.lock().unwrap_or_else(|p| p.into_inner()).replace(why.clone()).as_deref() == Some(why.as_str());
            let line = format!("Couldn't read Valve's events schedule: {why}");
            if again {
                logbus::debug("events", line);
            } else {
                logbus::warn("events", line);
            }
            EventList { error: Some(why), ..kept.unwrap_or_else(built_in) }
        }
    }
}

/// The list that comes with this version, for when the page was never read.
pub fn built_in() -> EventList {
    EventList { events: parse(BUILT_IN), fetched_at: None, from: "built-in".into(), error: None }
}

/// The list read last time, if there is one.
pub fn load(file: &Path) -> Option<EventList> {
    let text = std::fs::read_to_string(file).ok()?;
    serde_json::from_str::<EventList>(&text).ok().filter(|l| !l.events.is_empty())
}

pub fn save(file: &Path, list: &EventList) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(list)?)?;
    std::fs::rename(&tmp, file)?;
    Ok(())
}

// ------------------------------------------------------------------ the page --

/// Reads the events off the page. Valve writes them three ways:
/// - a heading with the dates after a bar: "Winter Sale 2026 | 17 December, 2026 - 4 January, 2027",
///   "Next Fest | June 14 - 21, 2027";
/// - a table, one row per fest, the dates on two lines ("Jan 14", "Jan 18"),
///   the year in the heading above ("2027 Fests");
/// - a sentence under a heading with a year: "Black Friday 2026 … From
///   November 26th 10AM PT through November 30th 10AM PT".
pub fn parse(html: &str) -> Vec<SteamEvent> {
    let doc = Html::parse_document(html);
    let Some(root) = doc.select(sel!("div.documentation_bbcode")).next() else {
        return Vec::new();
    };
    let mut page = Reader::default();
    for child in root.children() {
        match child.value() {
            Node::Text(text) => page.prose.push_str(text),
            Node::Element(element) => {
                let Some(el) = ElementRef::wrap(child) else { continue };
                match element.name() {
                    "h1" | "h2" | "h3" | "h4" => {
                        let main = element.name() == "h1" || element.has_class("bb_section", scraper::CaseSensitivity::AsciiCaseInsensitive);
                        page.heading(&squash(el.text()), main);
                    }
                    "table" => page.table(el),
                    _ => {
                        page.prose.push(' ');
                        page.prose.push_str(&el.text().collect::<String>());
                        page.prose.push(' ');
                    }
                }
            }
            _ => {}
        }
    }
    page.end_section();
    page.events
}

#[derive(Default)]
struct Reader {
    events: Vec<SteamEvent>,
    /// The heading of the section being read: "Seasonal Sales", "Black Friday 2026".
    section: String,
    /// Whether the section has given an event yet, by heading or table.
    section_dated: bool,
    /// The year the latest heading named ("2027 Fests", "2027").
    year: Option<i32>,
    /// The section's text, for a section that gives its dates in a sentence.
    prose: String,
}

impl Reader {
    fn heading(&mut self, text: &str, main: bool) {
        // A section's text runs on under its subheadings: "Black Friday 2026",
        // its sentence with the dates, then perhaps "How to take part".
        if main {
            self.end_section();
            self.section = text.to_string();
            self.section_dated = false;
        }
        if let Some((name, dates)) = text.split_once('|') {
            let name = squash_str(name);
            if let Some(y) = year_in(&name) {
                self.year = Some(y);
            }
            let kind = kind_of(&self.section, &name);
            if self.add(&name, kind, dates) {
                self.section_dated = true;
            }
        } else if let Some(y) = year_in(text) {
            self.year = Some(y);
        }
    }

    /// One fest per row: the dates from the column headed "...Dates", the
    /// name from the one headed "Theme" (or "Event", "Name"; else the first
    /// other column).
    fn table(&mut self, table: ElementRef) {
        let rows: Vec<ElementRef> = table.select(sel!("tr")).collect();
        let Some(head) = rows.first() else { return };
        let titles: Vec<String> = head.select(sel!("th, td")).map(|c| squash(c.text()).to_lowercase()).collect();
        let Some(dates_at) = titles.iter().position(|t| t.contains("date")) else { return };
        let Some(name_at) = titles
            .iter()
            .position(|t| !t.contains("date") && (t.contains("theme") || t.contains("event") || t.contains("name")))
            .or_else(|| (0..titles.len()).find(|&i| i != dates_at))
        else {
            return;
        };
        for row in &rows[1..] {
            let cells: Vec<ElementRef> = row.select(sel!("td")).collect();
            let (Some(dates), Some(name)) = (cells.get(dates_at), cells.get(name_at)) else { continue };
            // "Jan 14<br>Jan 18": the first day on one line, the last on the next.
            let lines: Vec<String> = dates.text().map(squash_str).filter(|l| !l.is_empty()).collect();
            let dates = match lines.as_slice() {
                [] => continue,
                [only] => only.clone(),
                [first, .., last] => format!("{first} - {last}"),
            };
            let name = squash(name.text());
            if name.is_empty() {
                continue;
            }
            let kind = kind_of(&self.section, &name);
            if self.add(&name, kind, &dates) {
                self.section_dated = true;
            }
        }
    }

    /// A section with no dates of its own may give them in a sentence: "From
    /// November 26th 10AM PT through November 30th 10AM PT", the year from
    /// its heading ("Black Friday 2026"), or in the sentence itself.
    fn end_section(&mut self) {
        let prose = std::mem::take(&mut self.prose);
        if self.section_dated || self.section.is_empty() {
            return;
        }
        let year = year_in(&self.section);
        let prose = squash_str(&prose);
        // "Games from every genre to celebrate" comes before the dates as
        // often as not: the first "from … to" that reads as dates counts.
        let range = re!(r"(?i)\bfrom\s+(.{3,40}?)\s+(?:through|until|to)\s+(.{3,40}?)(?:[,.;]|$)")
            .captures_iter(&prose)
            .find_map(|found| read_range(&format!("{} - {}", &found[1], &found[2]), year));
        if let Some(range) = range {
            let name = self.section.clone();
            let kind = kind_of(&self.section, &name);
            self.events.push(event(&name, kind, Some(range)));
            self.section_dated = true;
        }
    }

    /// Adds the event, if `dates` are dates or say they're still to be decided
    /// ("TBD"). Anything else after a bar ("Registration | Requirements") is
    /// not an event.
    fn add(&mut self, name: &str, kind: &'static str, dates: &str) -> bool {
        let range = read_range(dates, self.year);
        if range.is_none() && !re!(r"(?i)\b(tbd|tba|to be (announced|decided|determined|confirmed)|coming soon)\b").is_match(dates) {
            return false;
        }
        // The Next Fests are all called "Next Fest": their month tells them apart.
        let name = match (&range, kind) {
            (Some(r), "nextfest") if year_in(name).is_none() => format!("{}: {}", name, r.first.format("%B %Y")),
            _ => name.to_string(),
        };
        self.events.push(event(&name, kind, range));
        true
    }
}

fn kind_of(section: &str, name: &str) -> &'static str {
    let (section, name) = (section.to_lowercase(), name.to_lowercase());
    if name.contains("next fest") || section.contains("next fest") {
        "nextfest"
    } else if section.contains("seasonal") || re!(r"\bsale\b").is_match(&name) {
        "sale"
    } else if section.contains("fest") || name.contains("fest") {
        "fest"
    } else {
        "event"
    }
}

fn event(name: &str, kind: &str, range: Option<Range>) -> SteamEvent {
    SteamEvent {
        name: name.to_string(),
        kind: kind.to_string(),
        first_day: range.as_ref().map(|r| r.first.to_string()),
        last_day: range.as_ref().map(|r| r.last.to_string()),
        starts: range.as_ref().map(|r| pacific_ms(r.first, r.starts_at)),
        ends: range.as_ref().map(|r| pacific_ms(r.last, r.ends_at)),
    }
}

// ----------------------------------------------------------------- dates --

#[derive(Debug, PartialEq)]
struct Range {
    first: NaiveDate,
    last: NaiveDate,
    starts_at: NaiveTime,
    ends_at: NaiveTime,
}

/// One end of a date range, as far as it was written out.
#[derive(Debug, Default, Clone, Copy)]
struct End {
    month: Option<u32>,
    day: Option<u32>,
    year: Option<i32>,
    time: Option<NaiveTime>,
}

/// Steam's events change over at 10 AM Pacific time unless Valve says otherwise.
fn ten_am() -> NaiveTime {
    NaiveTime::from_hms_opt(10, 0, 0).expect("10:00")
}

/// Reads "17 December, 2026 - 4 January, 2027", "February 22 - March 1, 2027",
/// "June 14 - 21, 2027", "Jan 14 - Jan 18" (with the year from a heading) or
/// "November 26th 10AM PT - November 30th 10AM PT". A side without a month,
/// or a year, takes it from the other side; a range that runs into January
/// ends a year later. Anything else, "TBD" included, gives None.
fn read_range(text: &str, year: Option<i32>) -> Option<Range> {
    let text = squash_str(&text.replace(['\u{2013}', '\u{2014}'], " - "));
    let (a, b) = match re!(r"\s+-\s+|\s+(?:to|through|until)\s+").splitn(&text, 2).collect::<Vec<_>>()[..] {
        [a, b] => (read_end(a), read_end(b)),
        [one] => (read_end(one), read_end(one)),
        _ => return None,
    };
    let first_month = a.month.or(b.month)?;
    let last_month = b.month.unwrap_or(first_month);
    let (first_day, last_day) = (a.day?, b.day?);
    let wraps = last_month < first_month;
    let (first_year, last_year) = match (a.year, b.year) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (x, x + wraps as i32),
        (None, Some(y)) => (y - wraps as i32, y),
        (None, None) => {
            let y = year?;
            (y, y + wraps as i32)
        }
    };
    let first = NaiveDate::from_ymd_opt(first_year, first_month, first_day)?;
    let last = NaiveDate::from_ymd_opt(last_year, last_month, last_day)?;
    // A typo on the page, not an event: Steam's run for days, not months.
    if last < first || (last - first).num_days() > 62 {
        return None;
    }
    Some(Range { first, last, starts_at: a.time.unwrap_or_else(ten_am), ends_at: b.time.unwrap_or_else(ten_am) })
}

fn read_end(text: &str) -> End {
    let mut rest = text.to_lowercase();
    let mut end = End::default();
    if let Some(t) = re!(r"\b(\d{1,2})(?::(\d{2}))?\s*(am|pm)\b").captures(&rest) {
        let hour: u32 = t[1].parse().unwrap_or(0);
        let minute: u32 = t.get(2).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let hour = match (&t[3], hour) {
            ("am", 12) => 0,
            ("am", h) => h,
            ("pm", 12) => 12,
            (_, h) => h + 12,
        };
        end.time = NaiveTime::from_hms_opt(hour, minute, 0);
        let whole = t.get(0).expect("match").range();
        rest.replace_range(whole, " ");
    }
    if let Some(y) = re!(r"\b(20\d{2})\b").captures(&rest) {
        end.year = y[1].parse().ok();
        let whole = y.get(0).expect("match").range();
        rest.replace_range(whole, " ");
    }
    end.month = month_in(&rest);
    end.day = re!(r"\b(\d{1,2})(?:st|nd|rd|th)?\b").captures(&rest).and_then(|d| d[1].parse().ok()).filter(|d| (1..=31).contains(d));
    end
}

fn month_in(text: &str) -> Option<u32> {
    let found = re!(r"\b(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?").captures(text)?;
    let month = match &found[1] {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        _ => 12,
    };
    Some(month)
}

fn year_in(text: &str) -> Option<i32> {
    re!(r"\b(20\d{2})\b").captures(text).and_then(|y| y[1].parse().ok())
}

/// A time in Pacific time as ms since 1970. Daylight saving time there runs
/// from the second Sunday in March to the first Sunday in November, changing
/// at 2 AM, so it is decided by the date for any time from 2 AM on.
fn pacific_ms(date: NaiveDate, time: NaiveTime) -> i64 {
    let year = date.year();
    let dst_from = NaiveDate::from_weekday_of_month_opt(year, 3, Weekday::Sun, 2);
    let dst_until = NaiveDate::from_weekday_of_month_opt(year, 11, Weekday::Sun, 1);
    let dst = matches!((dst_from, dst_until), (Some(from), Some(until)) if date >= from && date < until);
    let offset_hours = if dst { 7 } else { 8 };
    (date.and_time(time) + chrono::Duration::hours(offset_hours)).and_utc().timestamp_millis()
}

fn squash<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    squash_str(&parts.collect::<String>())
}

fn squash_str(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn utc(text: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(text).unwrap().timestamp_millis()
    }

    #[test]
    fn reads_every_event_on_valves_page() {
        let events = parse(BUILT_IN);
        let names: Vec<&str> = events.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Winter Sale 2026",
                "Spring Sale 2027",
                "Summer Sale 2027",
                "Autumn Sale 2027",
                "Black Friday 2026",
                "Auto-Battler RPG Fest",
                "Desktop Companion Fest",
                "Shop Keeper Fest",
                "Sheep Fest",
                "Couch Co-Op Fest",
                "Rhythm Fest",
                "Dinos vs. Robots Fest",
                "Racing Fest",
                "Witch Fest",
                "Fighting Fest",
                "Real-Time Strategy Fest",
                "Mountaineering Fest",
                "Next Fest: February 2027",
                "Next Fest: June 2027",
            ]
        );
        let find = |name: &str| events.iter().find(|e| e.name == name).unwrap().clone();

        let winter = find("Winter Sale 2026");
        assert_eq!(winter.kind, "sale");
        assert_eq!((winter.first_day.as_deref(), winter.last_day.as_deref()), (Some("2026-12-17"), Some("2027-01-04")));
        // 10 AM Pacific in winter is 18:00 UTC.
        assert_eq!(winter.starts, Some(utc("2026-12-17T18:00:00Z")));
        assert_eq!(winter.ends, Some(utc("2027-01-04T18:00:00Z")));

        // Still to be decided: listed, without dates.
        let autumn = find("Autumn Sale 2027");
        assert_eq!((autumn.kind.as_str(), autumn.starts, autumn.first_day), ("sale", None, None));

        // Dates in a sentence, with their times.
        let black_friday = find("Black Friday 2026");
        assert_eq!(black_friday.kind, "event");
        assert_eq!(black_friday.starts, Some(utc("2026-11-26T18:00:00Z")));
        assert_eq!(black_friday.ends, Some(utc("2026-11-30T18:00:00Z")));

        // A table row, its year from the heading above, its end in the next month.
        let shop = find("Shop Keeper Fest");
        assert_eq!(shop.kind, "fest");
        assert_eq!((shop.first_day.as_deref(), shop.last_day.as_deref()), (Some("2027-01-25"), Some("2027-02-01")));
        let past = find("Auto-Battler RPG Fest");
        assert_eq!(past.first_day.as_deref(), Some("2026-11-16"));

        // After the clocks change: 10 AM Pacific is 17:00 UTC.
        let racing = find("Racing Fest");
        assert_eq!(racing.starts, Some(utc("2027-04-12T17:00:00Z")));

        let june = find("Next Fest: June 2027");
        assert_eq!(june.kind, "nextfest");
        assert_eq!((june.first_day.as_deref(), june.last_day.as_deref()), (Some("2027-06-14"), Some("2027-06-21")));
        let february = find("Next Fest: February 2027");
        assert_eq!((february.first_day.as_deref(), february.last_day.as_deref()), (Some("2027-02-22"), Some("2027-03-01")));
    }

    #[test]
    fn reads_the_ways_dates_are_written() {
        let range = |text: &str, year: Option<i32>| read_range(text, year).map(|r| (r.first, r.last));
        assert_eq!(range("17 December, 2026 - 4 January, 2027", None), Some((day(2026, 12, 17), day(2027, 1, 4))));
        assert_eq!(range("February 22 - March 1, 2027", None), Some((day(2027, 2, 22), day(2027, 3, 1))));
        assert_eq!(range("June 14 - 21, 2027", None), Some((day(2027, 6, 14), day(2027, 6, 21))));
        assert_eq!(range("Dec 28 \u{2013} Jan 4", Some(2027)), Some((day(2027, 12, 28), day(2028, 1, 4))));
        assert_eq!(range("December 28 - January 4, 2028", None), Some((day(2027, 12, 28), day(2028, 1, 4))));
        assert_eq!(range("Sept. 3 to Sept. 10", Some(2027)), Some((day(2027, 9, 3), day(2027, 9, 10))));
        assert_eq!(range("TBD", Some(2027)), None);
        // No year anywhere: not guessed.
        assert_eq!(range("Jan 14 - Jan 18", None), None);
        // Not an event's length.
        assert_eq!(range("January 1 - December 31, 2027", None), None);
        let timed = read_range("November 26th 10AM PT - November 30th 9:30 pm PT", Some(2026)).unwrap();
        assert_eq!((timed.starts_at, timed.ends_at), (ten_am(), NaiveTime::from_hms_opt(21, 30, 0).unwrap()));
    }

    #[test]
    fn pacific_time_follows_daylight_saving() {
        let ten = ten_am();
        // 2027: summer time from 14 March to 7 November.
        assert_eq!(pacific_ms(day(2027, 3, 13), ten), utc("2027-03-13T18:00:00Z"));
        assert_eq!(pacific_ms(day(2027, 3, 14), ten), utc("2027-03-14T17:00:00Z"));
        assert_eq!(pacific_ms(day(2027, 11, 6), ten), utc("2027-11-06T17:00:00Z"));
        assert_eq!(pacific_ms(day(2027, 11, 7), ten), utc("2027-11-07T18:00:00Z"));
    }

    /// Valve edits this page through the year; these are edits it could make
    /// that the reading must survive.
    #[test]
    fn dates_in_a_sentence_survive_a_subheading_after_them() {
        let page = r#"<div class="documentation_bbcode"><h2 class="bb_section">Black Friday 2027</h2>
            The hub returns. From November 25th 10AM PT through November 29th 10AM PT, discounts are featured.
            <h2 class="bb_subsection">How to take part</h2>Set up a discount as usual.</div>"#;
        let events = parse(page);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!((events[0].first_day.as_deref(), events[0].last_day.as_deref()), (Some("2027-11-25"), Some("2027-11-29")));
    }

    #[test]
    fn a_from_and_to_that_arent_dates_dont_hide_the_dates_after_them() {
        let page = r#"<div class="documentation_bbcode"><h2 class="bb_section">Lunar New Year 2027</h2>
            Games from every genre to celebrate the year. From February 4th through February 11th, the sale runs.</div>"#;
        let events = parse(page);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!((events[0].first_day.as_deref(), events[0].last_day.as_deref()), (Some("2027-02-04"), Some("2027-02-11")));
    }

    #[test]
    fn a_sentence_with_whole_dates_needs_no_year_in_its_heading() {
        let page = r#"<div class="documentation_bbcode"><h2 class="bb_section">Lunar New Year Sale</h2>
            From February 4th, 2027 through February 11th, 2027, discounts are featured.</div>"#;
        let events = parse(page);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!((events[0].first_day.as_deref(), events[0].last_day.as_deref()), (Some("2027-02-04"), Some("2027-02-11")));
    }

    #[test]
    fn a_fest_table_with_its_name_column_renamed_still_reads() {
        let page = r#"<div class="documentation_bbcode"><h2 class="bb_section">Themed Sale Events (Fests)</h2>
            <h2 class="bb_subsection">2027 Fests</h2><table><tr><th>Event Dates</th><th>Fest</th><th>Registration</th></tr>
            <tr><td>Jan 14<br>Jan 18</td><td>Desktop Companion Fest</td><td>Registration details</td></tr></table></div>"#;
        let names: Vec<String> = parse(page).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["Desktop Companion Fest"]);
    }

    #[test]
    fn a_heading_with_a_bar_but_no_dates_is_not_an_event() {
        let page = r#"<div class="documentation_bbcode"><h2 class="bb_section">Next Fest</h2>
            <h2 class="bb_subsection">Registration | Requirements</h2>Have a demo ready.
            <h2 class="bb_subsection">Next Fest | June 14 - 21, 2027</h2>
            <h2 class="bb_subsection">Next Fest | TBD</h2></div>"#;
        let names: Vec<String> = parse(page).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["Next Fest: June 2027", "Next Fest"]);
    }

    #[tokio::test]
    async fn the_page_is_only_read_when_asked_for() {
        let dir = std::env::temp_dir().join(format!("idlekit-events-list-{}", std::process::id()));
        let file = dir.join(FILE);
        let _ = std::fs::remove_dir_all(&dir);
        let read = std::sync::atomic::AtomicUsize::new(0);
        let page = || async {
            read.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(parse(BUILT_IN))
        };
        let down = || async {
            read.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(anyhow!("Valve's page couldn't be reached"))
        };
        let reads = || read.load(std::sync::atomic::Ordering::SeqCst);

        // Nothing kept: the list this version came with, at once, without the network.
        let first = list(&file, false, page).await;
        assert_eq!((first.from.as_str(), first.fetched_at, reads()), ("built-in", None, 0));

        // The page unreachable: what was there, and why; nothing written.
        let failed = list(&file, true, down).await;
        assert_eq!((failed.from.as_str(), failed.error.as_deref(), reads()), ("built-in", Some("Valve's page couldn't be reached"), 1));
        assert!(!file.exists());

        // Read: kept, and given without the network from then on.
        let fresh = list(&file, true, page).await;
        assert_eq!((fresh.from.as_str(), fresh.error.as_deref(), reads()), ("valve", None, 2));
        assert!(fresh.fetched_at.is_some());
        assert_eq!(list(&file, false, page).await, fresh);
        assert_eq!(reads(), 2);

        // Unreachable again: the list read before stays, with the reason.
        let kept = list(&file, true, down).await;
        assert_eq!((kept.fetched_at, kept.error.as_deref()), (fresh.fetched_at, Some("Valve's page couldn't be reached")));
        assert_eq!(kept.events, fresh.events);

        // And again, for the same reason: no new warning, only a debug line.
        list(&file, true, down).await;
        let said = |level: &str| {
            logbus::recent()
                .iter()
                .filter(|l| l.scope == "events" && l.level == level && l.message.ends_with("Valve's page couldn't be reached"))
                .count()
        };
        assert_eq!((said("warn"), said("debug")), (2, 1));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_page_without_the_list_gives_nothing() {
        assert!(parse("<html><body><p>Sign in</p></body></html>").is_empty());
        let changed = r#"<div class="documentation_bbcode"><h2 class="bb_section">Seasonal Sales</h2>Coming soon.</div>"#;
        assert!(parse(changed).is_empty());
    }

    #[test]
    fn a_kept_list_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("idlekit-events-{}", std::process::id()));
        let file = dir.join(FILE);
        assert_eq!(load(&file), None);
        let list = EventList { fetched_at: Some(1), from: "valve".into(), ..built_in() };
        save(&file, &list).unwrap();
        assert_eq!(load(&file), Some(list));
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(load(&file), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
