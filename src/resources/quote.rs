use crate::error::Result;
use crate::resources::DailyResource;
use askama::Template;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize, Serialize, Template)]
#[template(path = "qotd.html")]
pub struct Quote {
    pub text: String,
    pub author: String,
    pub work: Option<String>,
    pub date: Option<String>,
    pub language: String,
}

impl DailyResource for Quote {}

// Thin shim so main.rs call-sites are unchanged.
pub fn load_quotes() -> Result<Vec<Quote>> {
    let raw = std::fs::read_to_string("./quotes.json")?;
    serde_json::from_str::<Vec<Quote>>(&raw).map_err(Into::into)
}

pub fn add_quote(quote: Quote) -> Result<()> {
    let mut quotes = load_quotes()?;
    quotes.push(quote);
    let json = serde_json::to_string_pretty(&quotes)?;
    std::fs::write("./quotes.json", json)?;
    Ok(())
}

pub fn delete_quote(index: usize) -> Result<()> {
    let mut quotes = load_quotes()?;
    if index < quotes.len() {
        quotes.remove(index);
        let json = serde_json::to_string_pretty(&quotes)?;
        std::fs::write("./quotes.json", json)?;
    }
    Ok(())
}

pub fn unique_authors(quotes: &[Quote]) -> Vec<String> {
    let mut seen = HashSet::new();
    quotes
        .iter()
        .map(|q| q.author.clone())
        .filter(|a| !a.is_empty() && seen.insert(a.clone()))
        .collect()
}

pub fn unique_works(quotes: &[Quote]) -> Vec<String> {
    let mut seen = HashSet::new();
    quotes
        .iter()
        .filter_map(|q| q.work.clone())
        .filter(|w| !w.is_empty() && seen.insert(w.clone()))
        .collect()
}

pub fn render_quote(quote: &Quote) -> Result<String> {
    let rendered = quote.render()?;
    Ok(rendered)
}
