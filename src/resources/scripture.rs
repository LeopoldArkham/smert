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

    /// The text one verse per line, each split into runs so the divine
    /// names the KJV prints in capitals can be set in small caps.
    pub fn verses(&self) -> Vec<Vec<Run>> {
        self.text.lines().map(str::trim).filter(|l| !l.is_empty()).map(runs).collect()
    }

    pub fn bible_gateway_url(&self) -> String {
        format!(
            "https://www.biblegateway.com/passage/?search={}&version=KJV",
            urlencoding::encode(&self.reference())
        )
    }
}

/// A stretch of verse text; `small_caps` marks a divine name, already
/// lowercased after its first letter ("LORD" becomes "Lord").
#[derive(Debug, PartialEq)]
pub struct Run {
    pub text:       String,
    pub small_caps: bool,
}

/// Names the KJV sets in capitals for the Tetragrammaton.
const DIVINE_NAMES: [&str; 3] = ["LORD", "GOD", "JEHOVAH"];

/// Split a verse around its divine names, keeping a following possessive
/// ("LORD’S") with the name.
fn runs(verse: &str) -> Vec<Run> {
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut rest = verse;
    while !rest.is_empty() {
        let word_len = rest.find(|c: char| !c.is_alphabetic()).unwrap_or(rest.len());
        let (word, after) = rest.split_at(word_len.max(rest.chars().next().map_or(0, char::len_utf8)));
        if DIVINE_NAMES.contains(&word) {
            let possessive = ["\u{2019}S", "'S"].into_iter()
                .find(|p| after.strip_prefix(p).is_some_and(|a| !a.starts_with(char::is_alphabetic)));
            let (name, after) = match possessive {
                Some(p) => rest.split_at(word.len() + p.len()),
                None    => (word, after),
            };
            if !plain.is_empty() {
                out.push(Run { text: std::mem::take(&mut plain), small_caps: false });
            }
            let mut text = name[..1].to_string();
            text.push_str(&name[1..].to_lowercase());
            out.push(Run { text, small_caps: true });
            rest = after;
        } else {
            plain.push_str(word);
            rest = after;
        }
    }
    if !plain.is_empty() {
        out.push(Run { text: plain, small_caps: false });
    }
    out
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
    fetch(&reference_from_link(input)?).await
}

/// Fetch a passage, e.g. "John 3:16-17", in the KJV from bible-api.com.
async fn fetch(reference: &str) -> Result<Scripture> {

    #[derive(serde::Deserialize)]
    struct Passage { verses: Vec<Verse> }
    #[derive(serde::Deserialize)]
    struct Verse { book_name: String, chapter: u16, verse: u16, text: String }

    let url = format!(
        "https://bible-api.com/{}?translation=kjv",
        urlencoding::encode(reference),
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

    // One verse per line. Verse texts come padded, and with line breaks
    // that fall mid-phrase, so those are dropped.
    let text = verses
        .iter()
        .map(|v| v.text.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(Scripture {
        id:          0,
        book:        first.book_name.clone(),
        chapter:     first.chapter,
        verse_start: first.verse,
        verse_end:   (last.verse != first.verse).then_some(last.verse),
        text,
    })
}

/// Scriptures added before verse breaks were kept have a whole passage on
/// one line. Re-fetch those and add the breaks, but only where the words are
/// unchanged, so a text typed in by hand is never replaced.
pub async fn restore_verse_breaks(pool: &SqlitePool) -> Result<()> {
    let sql = format!(
        "{} WHERE verse_end IS NOT NULL AND instr(text, char(10)) = 0",
        Scripture::SELECT,
    );
    let flat: Vec<Scripture> = sqlx::query_as(&sql).fetch_all(pool).await?;
    let words = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
    for s in flat {
        let reference = format!("{} {}:{}-{}", s.book, s.chapter, s.verse_start, s.verse_end.unwrap_or(s.verse_start));
        match fetch(&reference).await {
            Ok(fetched) if words(&fetched.text) == words(&s.text) => {
                sqlx::query("UPDATE scriptures SET text = ? WHERE id = ?")
                    .bind(&fetched.text)
                    .bind(s.id)
                    .execute(pool)
                    .await?;
                println!("restored verse breaks in {reference}");
            }
            Ok(_)  => println!("not restoring verse breaks in {reference}: its text differs from the KJV"),
            Err(e) => println!("not restoring verse breaks in {reference}: {e}"),
        }
    }
    Ok(())
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
    use super::{Run, reference_from_link, runs};

    #[test]
    fn sets_divine_names_in_small_caps() {
        let run = |text: &str, small_caps| Run { text: text.into(), small_caps };
        assert_eq!(runs("The LORD is my shepherd; I shall not want."), [
            run("The ", false), run("Lord", true), run(" is my shepherd; I shall not want.", false),
        ]);
        assert_eq!(runs("the Lord GOD of the LORD\u{2019}S house"), [
            run("the Lord ", false), run("God", true), run(" of the ", false), run("Lord\u{2019}s", true), run(" house", false),
        ]);
        assert_eq!(runs("LORDS and GODLY men, O LORD"), [
            run("LORDS and GODLY men, O ", false), run("Lord", true),
        ]);
    }

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
