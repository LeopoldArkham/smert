use crate::error::Result;
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs::OpenOptions;

use askama::Template;

#[derive(Debug, Clone, Deserialize, Serialize, Template)]
#[template(path = "qotd.html")]
pub struct Quote {
    pub text: String,
    pub author: String,
    pub work: Option<String>,
}

pub fn load_quotes() -> Result<Vec<Quote>> {
    let raw = std::fs::read_to_string("./quotes.json")?;
    serde_json::from_str::<Vec<Quote>>(&raw).map_err(Into::into)
}

pub fn add_quote(quote: &Quote) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .append(true)
        .open("./quotes.json")?;

    if file.metadata()?.len() == 0 {
        serde_json::to_writer(&file, &vec![quote])?;
    } else {
        serde_json::to_writer(&file, &quote)?;
    }
    Ok(())
}

pub fn get_current_quote() -> Result<Quote> {
    let quotes = load_quotes()?;
    let Some(quote) = quotes.first() else {
        return Err("No quotes found".into());
    };
    Ok(quote.clone())
}

pub fn render_quote(quote: &Quote) -> Result<String> {
    let rendered = quote.render()?;
    Ok(rendered)
}
