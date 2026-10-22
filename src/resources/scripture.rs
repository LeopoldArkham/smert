use crate::error::Result;
use crate::resources::DailyResource;
use serde::{Deserialize, Serialize};

const DATA_FILE: &str = "./scriptures.json";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Scripture {
    pub book:        String,
    pub chapter:     u16,
    pub verse_start: u16,
    pub verse_end:   Option<u16>,
    pub text:        String,
}

impl Scripture {
    pub fn reference(&self) -> String {
        match self.verse_end {
            Some(end) => format!("{} {}:{}\u{2013}{}", self.book, self.chapter, self.verse_start, end),
            None      => format!("{} {}:{}", self.book, self.chapter, self.verse_start),
        }
    }

    pub fn bible_gateway_url(&self) -> String {
        format!(
            "https://www.biblegateway.com/passage/?search={}&version=KJV",
            urlencoding::encode(&self.reference())
        )
    }
}

impl DailyResource for Scripture {}

pub fn load_scriptures() -> Result<Vec<Scripture>> {
    if !std::path::Path::new(DATA_FILE).exists() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(DATA_FILE)?;
    serde_json::from_str::<Vec<Scripture>>(&raw).map_err(Into::into)
}

pub fn add_scripture(s: Scripture) -> Result<()> {
    let mut all = load_scriptures()?;
    all.push(s);
    std::fs::write(DATA_FILE, serde_json::to_string_pretty(&all)?)?;
    Ok(())
}

pub fn delete_scripture(index: usize) -> Result<()> {
    let mut all = load_scriptures()?;
    if index < all.len() {
        all.remove(index);
        std::fs::write(DATA_FILE, serde_json::to_string_pretty(&all)?)?;
    }
    Ok(())
}
