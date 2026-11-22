/// A text pattern built the first time its line runs, then reused. Building
/// one costs far more than using it, and the parsers run their lines in loops.
macro_rules! re {
    ($pattern:literal) => {{
        static BUILT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        BUILT.get_or_init(|| regex::Regex::new($pattern).expect("valid regex"))
    }};
}

/// The same for a CSS selector used to pick elements out of a page.
macro_rules! sel {
    ($css:literal) => {{
        static BUILT: std::sync::OnceLock<scraper::Selector> = std::sync::OnceLock::new();
        BUILT.get_or_init(|| scraper::Selector::parse($css).expect("valid selector"))
    }};
}

pub mod art;
pub mod auth;
pub mod cm;
pub mod community;
pub mod helper;
pub mod jwt;
pub mod kv_binary;
pub mod local;
pub mod market;
pub mod pictures;
pub mod schema;
pub mod vdf;
pub mod webapi;
