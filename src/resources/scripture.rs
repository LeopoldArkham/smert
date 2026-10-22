use crate::error::Result;
use crate::resources::Resource;
use sqlx::{SqliteExecutor, SqlitePool};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Scripture {
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

// ── Adding from a BibleGateway link ──────────────────────────────────────────

/// Build a scripture from a BibleGateway link (or a plain reference such as
/// "John 3:16-17"). The link only says which passage; the text is always
/// fetched in the KJV from bible-api.com, whatever version the link uses.
pub async fn from_link(input: &str) -> Result<Scripture> {
    let reference = reference_from_link(input)?;

    #[derive(serde::Deserialize)]
    struct Passage { verses: Vec<Verse> }
    #[derive(serde::Deserialize)]
    struct Verse { book_name: String, chapter: u16, verse: u16, text: String }

    let url = format!(
        "https://bible-api.com/{}?translation=kjv",
        urlencoding::encode(&reference),
    );
    let client = reqwest::Client::builder()
        .user_agent("smert/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let resp = client.get(&url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("\u{201c}{reference}\u{201d} wasn't found in the KJV").into());
    }
    if !resp.status().is_success() {
        return Err(format!("bible-api.com returned {}", resp.status()).into());
    }
    let verses = resp.json::<Passage>().await?.verses;

    let first = verses.first().ok_or("no verses found for this passage")?;
    if verses.iter().any(|v| v.book_name != first.book_name || v.chapter != first.chapter) {
        return Err("passages across several chapters aren't supported; add each chapter separately".into());
    }
    if verses.iter().zip(first.verse..).any(|(v, n)| v.verse != n) {
        return Err("only consecutive verses can be added as one scripture".into());
    }
    let last = verses.last().unwrap_or(first);

    // Verse texts come padded and with line breaks from the poetry layout.
    let text = verses
        .iter()
        .flat_map(|v| v.text.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ");

    Ok(Scripture {
        id:          0,
        book:        first.book_name.clone(),
        chapter:     first.chapter,
        verse_start: first.verse,
        verse_end:   (last.verse != first.verse).then_some(last.verse),
        text,
    })
}

/// The passage reference in a BibleGateway link: the `search` parameter of
/// /passage/ links, or the last path segment of /verse/<lang>/<reference>
/// links. Anything that isn't a link is taken as a reference itself.
fn reference_from_link(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("paste a BibleGateway link".into());
    }
    if !input.starts_with("http://") && !input.starts_with("https://") {
        return Ok(input.to_string());
    }
    let url = reqwest::Url::parse(input).map_err(|_| "this doesn't look like a valid link")?;
    if !url.host_str().is_some_and(|h| h == "biblegateway.com" || h.ends_with(".biblegateway.com")) {
        return Err("paste a BibleGateway link, or a reference like John 3:16".into());
    }
    if let Some((_, search)) = url.query_pairs().find(|(k, _)| k == "search") {
        let search = search.trim();
        if search.contains(';') {
            return Err("this link has several passages; add them one at a time".into());
        }
        if !search.is_empty() {
            return Ok(search.to_string());
        }
    }
    let segments: Vec<_> = url.path_segments().map(|s| s.filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    if segments.first() == Some(&"verse") {
        if let Some(last) = segments.last().filter(|_| segments.len() >= 2) {
            let reference = urlencoding::decode(last).map(|s| s.into_owned()).unwrap_or_else(|_| last.to_string());
            return Ok(reference.replace('+', " "));
        }
    }
    Err("couldn't find a passage in this link".into())
}

#[cfg(test)]
mod tests {
    use super::reference_from_link;

    #[test]
    fn reads_the_passage_from_biblegateway_links() {
        let cases = [
            ("https://www.biblegateway.com/passage/?search=John%203%3A16&version=KJV", "John 3:16"),
            ("https://www.biblegateway.com/passage/?search=Ephesians+6%3A12-13&version=NIV", "Ephesians 6:12-13"),
            ("https://www.biblegateway.com/passage/?version=ESV&search=1%20John%204", "1 John 4"),
            ("https://www.biblegateway.com/verse/en/Romans%208%3A28", "Romans 8:28"),
            ("  John 3:16  ", "John 3:16"),
        ];
        for (input, expected) in cases {
            assert_eq!(reference_from_link(input).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn rejects_other_links_and_multiple_passages() {
        assert!(reference_from_link("https://example.com/?search=John%203:16").is_err());
        assert!(reference_from_link("https://www.biblegateway.com/passage/?search=John%203%3A16%3B%20Romans%208%3A28").is_err());
        assert!(reference_from_link("https://www.biblegateway.com/").is_err());
        assert!(reference_from_link("").is_err());
    }
}
