use crate::error::Result;
use crate::resources::Resource;
use serde::Serialize;
use sqlx::{SqliteExecutor, SqlitePool};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Quote {
    pub id: i64,
    pub text: String,
    pub author: String,
    pub work: Option<String>,
    pub date: Option<String>,
    pub language: String,
}

impl Resource for Quote {
    const SELECT: &'static str = "SELECT id, text, author, work, date, language FROM quotes";
}

/// All quotes, newest first.
pub async fn all(pool: &SqlitePool) -> Result<Vec<Quote>> {
    let sql = format!("{} ORDER BY id DESC", Quote::SELECT);
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

pub async fn add<'e>(db: impl SqliteExecutor<'e>, quote: &Quote) -> Result<()> {
    sqlx::query("INSERT INTO quotes (text, author, work, date, language) VALUES (?, ?, ?, ?, ?)")
        .bind(&quote.text)
        .bind(&quote.author)
        .bind(&quote.work)
        .bind(&quote.date)
        .bind(&quote.language)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM quotes WHERE id = ?").bind(id).execute(pool).await?;
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
