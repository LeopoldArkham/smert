//! One-off import of the legacy JSON files into the database:
//! `smert import [DIR]`, where DIR (default `.`) holds the JSON files and
//! `static/artwork/`. Artwork images are copied into the data folder.

use crate::error::Result;
use crate::resources::artwork::{self, NewArtwork};
use crate::resources::definition::{self, Definition};
use crate::resources::quote::{self, Quote};
use crate::resources::scripture::{self, Scripture};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sqlx::SqlitePool;
use std::path::Path;

/// An entry of the legacy artworks.json, whose image lives at
/// `static/artwork/<filename>` and thumbnail at `static/artwork/thumbs/<thumbnail>`.
#[derive(Deserialize)]
struct LegacyArtwork {
    url:       String,
    filename:  String,
    thumbnail: Option<String>,
    artist:    String,
    work:      String,
    date:      String,
}

pub async fn run(pool: &SqlitePool, images_dir: &Path, dir: &Path) -> Result<()> {
    let quotes:     Vec<Quote>         = read(dir, "quotes.json")?;
    let artworks:   Vec<LegacyArtwork> = read(dir, "artworks.json")?;
    let defs_en:    Vec<Definition>    = read(dir, "definitions_en.json")?;
    let defs_fr:    Vec<Definition>    = read(dir, "definitions_fr.json")?;
    let scriptures: Vec<Scripture>     = read(dir, "scriptures.json")?;

    let mut tx = pool.begin().await?;

    let existing: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM quotes) + (SELECT COUNT(*) FROM artworks)
              + (SELECT COUNT(*) FROM definitions) + (SELECT COUNT(*) FROM scriptures)",
    )
    .fetch_one(&mut *tx)
    .await?;
    if existing > 0 {
        return Err("the database already has content; refusing to import on top of it".into());
    }

    // Insert in file order, so ids (and the admin's newest-first lists) follow
    // the order items were originally added in.
    for q in &quotes     { quote::add(&mut *tx, q).await?; }
    let legacy_images = dir.join("static/artwork");
    for (i, a) in artworks.iter().enumerate() {
        let image = std::fs::read(legacy_images.join(&a.filename))
            .map_err(|e| format!("{}: {e}", legacy_images.join(&a.filename).display()))?;
        let thumbnail = a.thumbnail.as_ref()
            .and_then(|t| std::fs::read(legacy_images.join("thumbs").join(t)).ok());
        let ext = a.filename.rsplit_once('.').map_or("jpg", |(_, ext)| ext);
        let new = NewArtwork { url: &a.url, artist: &a.artist, work: &a.work, date: &a.date };
        artwork::add(&mut tx, images_dir, &new, ext, &image, thumbnail.as_deref()).await?;
        println!("  artwork {}/{}: {}", i + 1, artworks.len(), a.work);
    }
    for d in &defs_en    { definition::add(&mut *tx, "english", d).await?; }
    for d in &defs_fr    { definition::add(&mut *tx, "french", d).await?; }
    for s in &scriptures { scripture::add(&mut *tx, s).await?; }

    tx.commit().await?;

    println!(
        "Imported {} quotes, {} artworks, {} English and {} French words, {} scriptures.",
        quotes.len(), artworks.len(), defs_en.len(), defs_fr.len(), scriptures.len(),
    );
    Ok(())
}

/// Parse `dir/name`, treating a missing file as empty.
fn read<T: DeserializeOwned>(dir: &Path, name: &str) -> Result<Vec<T>> {
    let path = dir.join(name);
    if !path.exists() {
        println!("{} not found, skipping.", path.display());
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(&path)?;
    serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()).into())
}
