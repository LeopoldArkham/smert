//! One-off import of the legacy JSON files into the database:
//! `smert import [DIR]`, where DIR (default `.`) holds the JSON files.

use crate::error::Result;
use crate::resources::artwork::{self, Artwork};
use crate::resources::definition::{self, Definition};
use crate::resources::quote::{self, Quote};
use crate::resources::scripture::{self, Scripture};
use serde::de::DeserializeOwned;
use sqlx::SqlitePool;
use std::path::Path;

pub async fn run(pool: &SqlitePool, dir: &Path) -> Result<()> {
    let quotes:     Vec<Quote>      = read(dir, "quotes.json")?;
    let artworks:   Vec<Artwork>    = read(dir, "artworks.json")?;
    let defs_en:    Vec<Definition> = read(dir, "definitions_en.json")?;
    let defs_fr:    Vec<Definition> = read(dir, "definitions_fr.json")?;
    let scriptures: Vec<Scripture>  = read(dir, "scriptures.json")?;

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
    for a in &artworks   { artwork::add(&mut *tx, a).await?; }
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
