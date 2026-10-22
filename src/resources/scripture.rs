use crate::error::Result;
use crate::resources::Resource;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteExecutor, SqlitePool};

#[derive(Debug, Clone, Deserialize, Serialize, sqlx::FromRow)]
pub struct Scripture {
    /// Absent in the legacy JSON files; assigned by the database.
    #[serde(default)]
    pub id:          i64,
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

impl Resource for Scripture {
    const SELECT: &'static str =
        "SELECT id, book, chapter, verse_start, verse_end, text FROM scriptures";
}

/// All scriptures, newest first.
pub async fn all(pool: &SqlitePool) -> Result<Vec<Scripture>> {
    let sql = format!("{} ORDER BY id DESC", Scripture::SELECT);
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

pub async fn add<'e>(db: impl SqliteExecutor<'e>, s: &Scripture) -> Result<()> {
    sqlx::query(
        "INSERT INTO scriptures (book, chapter, verse_start, verse_end, text) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&s.book)
    .bind(s.chapter)
    .bind(s.verse_start)
    .bind(s.verse_end)
    .bind(&s.text)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM scriptures WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}
