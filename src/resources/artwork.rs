use crate::error::Result;
use crate::resources::Resource;
use askama::Template;
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};
use std::path::{Path, PathBuf};

/// Max width (px) requested when importing a Commons image — keeps downloads
/// sane while staying high-resolution. Originals can be hundreds of MB.
const MAX_IMAGE_WIDTH: u32 = 3840;
/// Hard cap on a single image download, so one oversized file can't OOM the
/// whole server. 64 MB comfortably covers a 4K-wide JPEG.
const MAX_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Template, sqlx::FromRow)]
#[template(path = "artwork.html")]
pub struct Artwork {
    pub id:        i64,
    /// Extension of the stored image file, e.g. "jpg".
    pub ext:       String,
    pub has_thumb: bool,
    pub artist:    String,
    pub work:      String,
    pub date:      String,
}

impl Resource for Artwork {
    const SELECT: &'static str =
        "SELECT id, ext, has_thumb, artist, work, date FROM artworks";
}

impl Artwork {
    /// Ends in the download name, so "Save image as" suggests it in every browser.
    pub fn image_url(&self) -> String {
        format!("/images/{}/{}", self.id, urlencoding::encode(&self.download_name()))
    }

    pub fn thumb_url(&self) -> String {
        if self.has_thumb { format!("/thumbs/{}", self.id) } else { self.image_url() }
    }

    /// "Artist - Work (Date).ext", leaving out whatever is empty, with
    /// characters that are invalid in filenames replaced.
    pub fn download_name(&self) -> String {
        let mut name = [self.artist.trim(), self.work.trim()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" - ");
        if !self.date.trim().is_empty() {
            name = format!("{name} ({})", self.date.trim());
        }
        let name: String = name
            .chars()
            .map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) { '_' } else { c })
            .collect();
        let stem = if name.trim().is_empty() { format!("artwork-{}", self.id) } else { name };
        format!("{stem}.{}", self.ext)
    }

    pub fn image_path(&self, images_dir: &Path) -> PathBuf {
        image_path(images_dir, self.id, &self.ext)
    }

    pub fn thumb_path(&self, images_dir: &Path) -> PathBuf {
        thumb_path(images_dir, self.id)
    }
}

/// An artwork that isn't stored yet.
pub struct NewArtwork<'a> {
    pub url:    &'a str,
    pub artist: &'a str,
    pub work:   &'a str,
    pub date:   &'a str,
}

fn image_path(images_dir: &Path, id: i64, ext: &str) -> PathBuf {
    images_dir.join(format!("{id}.{ext}"))
}

fn thumb_path(images_dir: &Path, id: i64) -> PathBuf {
    images_dir.join("thumbs").join(format!("{id}.jpg"))
}

/// Metadata resolved from a Wikipedia / Wikidata URL, ready to pre-fill the
/// manual "add artwork" form. `url` is the highest-resolution image URL.
#[derive(Debug, Serialize)]
pub struct ResolvedArtwork {
    pub url:    String,
    pub artist: String,
    pub work:   String,
    pub date:   String,
}

/// All artworks, newest first.
pub async fn all(pool: &SqlitePool) -> Result<Vec<Artwork>> {
    let sql = format!("{} ORDER BY id DESC", Artwork::SELECT);
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

/// Insert the artwork, write its image file and generate its thumbnail. Runs
/// on `conn` so a caller's transaction also undoes the insert if a write fails.
pub async fn add(
    conn:       &mut SqliteConnection,
    images_dir: &Path,
    new:        &NewArtwork<'_>,
    ext:        &str,
    image:      &[u8],
) -> Result<i64> {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO artworks (url, ext, artist, work, date) VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(new.url)
    .bind(ext)
    .bind(new.artist)
    .bind(new.work)
    .bind(new.date)
    .fetch_one(&mut *conn)
    .await?;

    std::fs::create_dir_all(images_dir.join("thumbs"))?;
    std::fs::write(image_path(images_dir, id, ext), image)?;

    let thumb = thumb_path(images_dir, id);
    let has_thumb = make_thumbnail(image, &thumb).is_ok();
    sqlx::query("UPDATE artworks SET has_thumb = ? WHERE id = ?")
        .bind(has_thumb)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(id)
}

pub async fn download_and_add(pool: &SqlitePool, images_dir: &Path, new: &NewArtwork<'_>) -> Result<()> {
    let url = new.url;
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()?;

    let mut response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()).into());
    }

    // Guard against giant downloads (Commons masters can be hundreds of MB):
    // reject early on the declared length so a single import can't OOM the server.
    if let Some(len) = response.content_length() {
        if len > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "image is {} MB, over the {} MB limit — request a smaller resolution",
                len / 1_048_576,
                MAX_DOWNLOAD_BYTES / 1_048_576,
            )
            .into());
        }
    }

    let ext = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| {
            let ct = ct.split(';').next().unwrap_or("").trim();
            match ct {
                "image/jpeg" | "image/jpg" => Some("jpg"),
                "image/png"                => Some("png"),
                "image/webp"               => Some("webp"),
                "image/gif"                => Some("gif"),
                "image/avif"               => Some("avif"),
                _                          => None,
            }
        })
        .unwrap_or_else(|| ext_from_url(url));

    // Stream the body with the same cap, in case Content-Length was absent.
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() as u64 + chunk.len() as u64 > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "image exceeds the {} MB limit — request a smaller resolution",
                MAX_DOWNLOAD_BYTES / 1_048_576,
            )
            .into());
        }
        bytes.extend_from_slice(&chunk);
    }

    let mut tx = pool.begin().await?;
    add(&mut tx, images_dir, new, ext, &bytes).await?;
    tx.commit().await?;
    Ok(())
}

/// Resolve artwork metadata + a highest-resolution image URL from a Wikipedia
/// or Wikidata link. Wikipedia pages are mapped to their backing Wikidata item;
/// the item's creator (P170), inception (P571) and image (P18) are then read.
pub async fn resolve_from_wiki(url: &str) -> Result<ResolvedArtwork> {
    let client = wiki_client()?;

    let qid = entity_id_for_url(&client, url).await?;
    let entity = fetch_entity(&client, &qid).await?;

    let work = pick_label(&entity).unwrap_or_default();
    let date = claim_year(&entity, "P571");

    let filename = claim_string(&entity, "P18")
        .ok_or("this Wikidata item has no image (P18)")?;
    // Commons downscales to `width` server-side (and serves the original when it
    // is smaller). The full master can be hundreds of MB, so cap at 4K — which
    // matches the existing catalogue's 3840px convention.
    let image_url = format!(
        "https://commons.wikimedia.org/wiki/Special:FilePath/{}?width={}",
        urlencoding::encode(&filename),
        MAX_IMAGE_WIDTH,
    );

    let artist = match claim_entity_id(&entity, "P170") {
        Some(creator_qid) => {
            let creator = fetch_entity(&client, &creator_qid).await?;
            pick_label(&creator).unwrap_or_default()
        }
        None => String::new(),
    };

    Ok(ResolvedArtwork { url: image_url, artist, work, date })
}

fn wiki_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("smert/0.1 (artwork importer; leopold@recovr.eu)")
        .build()
        .map_err(Into::into)
}

fn split_host_path(url: &str) -> (String, String) {
    let after = url.splitn(2, "://").nth(1).unwrap_or(url);
    match after.find('/') {
        Some(i) => (after[..i].to_string(), after[i..].to_string()),
        None    => (after.to_string(), String::new()),
    }
}

/// Determine the Wikidata Q-id for a Wikipedia article or Wikidata URL.
async fn entity_id_for_url(client: &reqwest::Client, url: &str) -> Result<String> {
    let (host, path) = split_host_path(url);
    let seg = path
        .strip_prefix("/wiki/")
        .map(|s| s.split(['#', '?']).next().unwrap_or(s));

    if host.contains("wikidata.org") {
        let seg = seg.ok_or("unrecognized Wikidata URL")?;
        let qid = seg.rsplit('/').next().unwrap_or(seg).trim_end_matches(".json");
        if qid.starts_with('Q') && qid[1..].chars().all(|c| c.is_ascii_digit()) {
            return Ok(qid.to_string());
        }
        return Err("could not find a Q-id in the Wikidata URL".into());
    }

    if host.contains("wikipedia.org") {
        let lang = host.split('.').next().unwrap_or("en");
        let seg = seg.ok_or("unrecognized Wikipedia URL")?;
        let title = urlencoding::decode(seg)
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| seg.to_string());
        let api = format!(
            "https://{}.wikipedia.org/w/api.php?action=query&format=json\
             &prop=pageprops&ppprop=wikibase_item&redirects=1&titles={}",
            lang,
            urlencoding::encode(&title),
        );
        let v: serde_json::Value = client.get(&api).send().await?.json().await?;
        let qid = v["query"]["pages"]
            .as_object()
            .and_then(|m| m.values().next())
            .and_then(|p| p["pageprops"]["wikibase_item"].as_str())
            .ok_or("no linked Wikidata item for this Wikipedia page")?;
        return Ok(qid.to_string());
    }

    Err("unsupported link — paste a Wikipedia or Wikidata URL".into())
}

async fn fetch_entity(client: &reqwest::Client, qid: &str) -> Result<serde_json::Value> {
    let url = format!("https://www.wikidata.org/wiki/Special:EntityData/{qid}.json");
    let v: serde_json::Value = client.get(&url).send().await?.json().await?;
    Ok(v["entities"][qid].clone())
}

/// Preferred display label for an entity: English, then French, then any.
fn pick_label(entity: &serde_json::Value) -> Option<String> {
    let labels = &entity["labels"];
    for lang in ["en", "fr"] {
        if let Some(s) = labels[lang]["value"].as_str() {
            return Some(s.to_string());
        }
    }
    labels
        .as_object()
        .and_then(|m| m.values().next())
        .and_then(|l| l["value"].as_str())
        .map(|s| s.to_string())
}

/// First value of a string-typed claim (e.g. P18 → Commons filename).
fn claim_string(entity: &serde_json::Value, prop: &str) -> Option<String> {
    entity["claims"][prop][0]["mainsnak"]["datavalue"]["value"]
        .as_str()
        .map(|s| s.to_string())
}

/// First value of an entity-typed claim (e.g. P170 → creator Q-id).
fn claim_entity_id(entity: &serde_json::Value, prop: &str) -> Option<String> {
    entity["claims"][prop][0]["mainsnak"]["datavalue"]["value"]["id"]
        .as_str()
        .map(|s| s.to_string())
}

/// Year of a time-typed claim (e.g. P571 inception), formatted like "1889" or
/// "44 BC". Empty string if absent.
fn claim_year(entity: &serde_json::Value, prop: &str) -> String {
    let time = entity["claims"][prop][0]["mainsnak"]["datavalue"]["value"]["time"]
        .as_str()
        .unwrap_or("");
    let bc = time.starts_with('-');
    let body = time.trim_start_matches(['+', '-']);
    let year = body.split('-').next().unwrap_or("").trim_start_matches('0');
    if year.is_empty() {
        String::new()
    } else if bc {
        format!("{year} BC")
    } else {
        year.to_string()
    }
}

pub async fn delete(pool: &SqlitePool, images_dir: &Path, id: i64) -> Result<()> {
    let Some(a) = super::get::<Artwork>(pool, id).await? else {
        return Ok(());
    };
    sqlx::query("DELETE FROM artworks WHERE id = ?").bind(id).execute(pool).await?;
    let _ = std::fs::remove_file(a.image_path(images_dir));
    let _ = std::fs::remove_file(a.thumb_path(images_dir));
    Ok(())
}


pub fn unique_artists(artworks: &[Artwork]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    artworks.iter()
        .filter(|a| seen.insert(a.artist.as_str()))
        .map(|a| a.artist.clone())
        .collect()
}

pub fn render_artwork(artwork: &Artwork) -> Result<String> {
    Ok(artwork.render()?)
}

/// Resize `image` to fit within 160×120 and save it to `dest` as JPEG.
fn make_thumbnail(image: &[u8], dest: &Path) -> Result<()> {
    let thumb = image::load_from_memory(image)?.thumbnail(160, 120);
    thumb.into_rgb8().save_with_format(dest, image::ImageFormat::Jpeg)?;
    Ok(())
}

fn ext_from_url(url: &str) -> &'static str {
    let path = url.split('?').next().unwrap_or(url);
    let path = path.split('#').next().unwrap_or(path);
    let last = path.rsplit('/').next().unwrap_or("");
    match last.rsplit('.').next().map(|e| e.to_lowercase()).as_deref() {
        Some("jpg") | Some("jpeg") => "jpg",
        Some("png")                => "png",
        Some("webp")               => "webp",
        Some("gif")                => "gif",
        Some("avif")               => "avif",
        _                          => "jpg",
    }
}
