//! Reads a game's achievement/stat definitions from the local Steam cache
//! (`<Steam>/appcache/stats/UserGameStatsSchema_<appid>.bin`). The Steamworks
//! API can't list stat names or "protected" flags; this file has both.
use super::kv_binary::{self, Kv};
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SchemaAchievement {
    pub api_name: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub icon_gray: String,
    pub hidden: bool,
    /// Non-zero permission means only the game's servers may set it ("protected").
    pub permission: i64,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SchemaStat {
    pub name: String,
    pub display_name: String,
    /// "int" | "float" | "avgrate"
    pub kind: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub max_change: Option<f64>,
    pub default: Option<f64>,
    pub increment_only: bool,
    pub permission: i64,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameSchema {
    pub appid: u32,
    pub game_name: String,
    pub achievements: Vec<SchemaAchievement>,
    pub stats: Vec<SchemaStat>,
}

fn localized(node: Option<&Kv>, language: &str, fallback: &str) -> String {
    match node {
        Some(Kv::Obj(_)) => {
            let n = node.unwrap();
            n.get(language)
                .or_else(|| n.get("english"))
                .and_then(Kv::as_string)
                .or_else(|| n.children().first().and_then(|(_, v)| v.as_string()))
                .unwrap_or_else(|| fallback.to_string())
        }
        Some(other) => other.as_string().unwrap_or_else(|| fallback.to_string()),
        None => fallback.to_string(),
    }
}

pub fn schema_path(steam_path: &Path, appid: u32) -> std::path::PathBuf {
    steam_path
        .join("appcache")
        .join("stats")
        .join(format!("UserGameStatsSchema_{appid}.bin"))
}

pub fn parse_schema(data: &[u8], appid: u32, language: &str) -> Result<GameSchema> {
    let root = kv_binary::parse(data)?;
    let game = root
        .get(&appid.to_string())
        .or_else(|| root.children().first().map(|(_, v)| v))
        .context("schema has no game section")?;
    let mut schema = GameSchema {
        appid,
        game_name: game.get("gamename").and_then(Kv::as_string).unwrap_or_default(),
        ..Default::default()
    };
    let Some(stats) = game.get("stats") else {
        return Ok(schema);
    };
    for (_, stat) in stats.children() {
        let ty = stat
            .get("type_int")
            .or_else(|| stat.get("type"))
            .and_then(|t| match t {
                Kv::Str(s) => match s.to_ascii_uppercase().as_str() {
                    "INT" => Some(1),
                    "FLOAT" => Some(2),
                    "AVGRATE" => Some(3),
                    "ACHIEVEMENTS" => Some(4),
                    "GROUPACHIEVEMENTS" => Some(5),
                    other => other.parse().ok(),
                },
                other => other.as_i64(),
            })
            .unwrap_or(0);
        match ty {
            1..=3 => {
                let name = stat.get("name").and_then(Kv::as_string).unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                schema.stats.push(SchemaStat {
                    display_name: localized(stat.path(&["display", "name"]), language, &name),
                    kind: match ty {
                        1 => "int",
                        2 => "float",
                        _ => "avgrate",
                    }
                    .into(),
                    min: stat.get("min").and_then(Kv::as_f64),
                    max: stat.get("max").and_then(Kv::as_f64),
                    max_change: stat.get("maxchange").and_then(Kv::as_f64),
                    default: stat.get("default").and_then(Kv::as_f64),
                    increment_only: stat.get("incrementonly").and_then(Kv::as_bool).unwrap_or(false),
                    permission: stat.get("permission").and_then(Kv::as_i64).unwrap_or(0),
                    name,
                });
            }
            4 | 5 => {
                for (key, bits) in stat.children() {
                    if !key.eq_ignore_ascii_case("bits") {
                        continue;
                    }
                    for (_, bit) in bits.children() {
                        let api_name = bit.get("name").and_then(Kv::as_string).unwrap_or_default();
                        if api_name.is_empty() {
                            continue;
                        }
                        let display = bit.get("display");
                        schema.achievements.push(SchemaAchievement {
                            name: localized(display.and_then(|d| d.get("name")), language, &api_name),
                            description: localized(display.and_then(|d| d.get("desc")), language, ""),
                            icon: display
                                .and_then(|d| d.get("icon"))
                                .and_then(Kv::as_string)
                                .unwrap_or_default(),
                            icon_gray: display
                                .and_then(|d| d.get("icon_gray"))
                                .and_then(Kv::as_string)
                                .unwrap_or_default(),
                            hidden: display
                                .and_then(|d| d.get("hidden"))
                                .and_then(Kv::as_bool)
                                .unwrap_or(false),
                            permission: bit.get("permission").and_then(Kv::as_i64).unwrap_or(0),
                            api_name,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    Ok(schema)
}

pub fn load(steam_path: &Path, appid: u32, language: &str) -> Result<GameSchema> {
    let path = schema_path(steam_path, appid);
    let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    parse_schema(&data, appid, language)
}

/// Steam CDN URL for an achievement icon hash from the schema.
pub fn icon_url(appid: u32, icon: &str) -> String {
    if icon.is_empty() || icon.starts_with("http") {
        return icon.to_string();
    }
    format!("https://cdn.steamstatic.com/steamcommunity/public/images/apps/{appid}/{icon}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steam::kv_binary::write;

    fn s(v: &str) -> Kv {
        Kv::Str(v.into())
    }

    #[test]
    fn parses_achievements_and_stats() {
        let doc = vec![(
            "480".to_string(),
            Kv::Obj(vec![
                ("gamename".into(), s("Spacewar")),
                (
                    "stats".into(),
                    Kv::Obj(vec![
                        (
                            "1".into(),
                            Kv::Obj(vec![
                                ("type".into(), s("4")),
                                (
                                    "bits".into(),
                                    Kv::Obj(vec![
                                        (
                                            "0".into(),
                                            Kv::Obj(vec![
                                                ("name".into(), s("ACH_WIN_ONE_GAME")),
                                                (
                                                    "display".into(),
                                                    Kv::Obj(vec![
                                                        (
                                                            "name".into(),
                                                            Kv::Obj(vec![
                                                                ("english".into(), s("Winner")),
                                                                ("german".into(), s("Gewinner")),
                                                            ]),
                                                        ),
                                                        ("desc".into(), Kv::Obj(vec![("english".into(), s("Win one game"))])),
                                                        ("hidden".into(), s("0")),
                                                        ("icon".into(), s("abc.jpg")),
                                                        ("icon_gray".into(), s("abc_gray.jpg")),
                                                    ]),
                                                ),
                                                ("bit".into(), Kv::Int(0)),
                                            ]),
                                        ),
                                        (
                                            "1".into(),
                                            Kv::Obj(vec![
                                                ("name".into(), s("ACH_PROTECTED")),
                                                ("permission".into(), Kv::Int(2)),
                                                ("display".into(), Kv::Obj(vec![("name".into(), s("Locked down")), ("hidden".into(), s("1"))])),
                                            ]),
                                        ),
                                    ]),
                                ),
                            ]),
                        ),
                        (
                            "2".into(),
                            Kv::Obj(vec![
                                ("type_int".into(), Kv::Int(1)),
                                ("name".into(), s("NumGames")),
                                ("display".into(), Kv::Obj(vec![("name".into(), s("Games played"))])),
                                ("min".into(), s("0")),
                                ("max".into(), s("1000")),
                                ("incrementonly".into(), s("1")),
                            ]),
                        ),
                        (
                            "3".into(),
                            Kv::Obj(vec![("type".into(), s("FLOAT")), ("name".into(), s("FeetTraveled"))]),
                        ),
                    ]),
                ),
            ]),
        )];
        let mut bytes = Vec::new();
        write(&doc, &mut bytes);
        bytes.push(8);

        let schema = parse_schema(&bytes, 480, "german").unwrap();
        assert_eq!(schema.game_name, "Spacewar");
        assert_eq!(schema.achievements.len(), 2);
        assert_eq!(schema.achievements[0].api_name, "ACH_WIN_ONE_GAME");
        assert_eq!(schema.achievements[0].name, "Gewinner");
        assert_eq!(schema.achievements[0].description, "Win one game");
        assert_eq!(schema.achievements[0].permission, 0);
        assert_eq!(schema.achievements[1].permission, 2);
        assert!(schema.achievements[1].hidden);
        assert_eq!(schema.achievements[1].name, "Locked down");
        assert_eq!(schema.stats.len(), 2);
        assert_eq!(schema.stats[0].name, "NumGames");
        assert_eq!(schema.stats[0].kind, "int");
        assert!(schema.stats[0].increment_only);
        assert_eq!(schema.stats[0].max, Some(1000.0));
        assert_eq!(schema.stats[1].kind, "float");
        assert_eq!(schema.stats[1].display_name, "FeetTraveled");
    }
}
