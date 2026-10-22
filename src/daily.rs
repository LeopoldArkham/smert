//! Daily selections, persisted in `daily_picks`.
//!
//! There is no scheduler: the first request after the day rolls over makes
//! the new pick, so restarts and downtime never skip or change a pick.

use crate::error::Result;
use crate::resources::{self, Resource};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use sqlx::SqlitePool;

const TIMEZONE: Tz = chrono_tz::Europe::Paris;
/// Local hour at which a new day's picks begin.
const ROLLOVER_HOUR: i64 = 5;

#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Quote,
    Artwork,
    WordEn,
    WordFr,
    Scripture,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Quote     => "quote",
            Kind::Artwork   => "artwork",
            Kind::WordEn    => "word_en",
            Kind::WordFr    => "word_fr",
            Kind::Scripture => "scripture",
        }
    }

    /// Query listing the ids this kind picks from.
    fn candidates(self) -> &'static str {
        match self {
            Kind::Quote     => "SELECT id FROM quotes",
            Kind::Artwork   => "SELECT id FROM artworks",
            Kind::WordEn    => "SELECT id FROM definitions WHERE language = 'english'",
            Kind::WordFr    => "SELECT id FROM definitions WHERE language = 'french'",
            Kind::Scripture => "SELECT id FROM scriptures",
        }
    }
}

/// The current "day" in Paris, which starts at 05:00 local time rather than
/// midnight. Naive local arithmetic keeps the boundary at 05:00 across DST.
pub fn today() -> NaiveDate {
    day_at(Utc::now())
}

fn day_at(now: DateTime<Utc>) -> NaiveDate {
    let local = now.with_timezone(&TIMEZONE).naive_local();
    (local - Duration::hours(ROLLOVER_HOUR)).date()
}

/// Today's pick for `kind`, loaded from its table. Makes the pick first if
/// there is none yet today, or if every pick today has since been deleted.
pub async fn todays<T: Resource>(pool: &SqlitePool, kind: Kind) -> Result<Option<T>> {
    let day = today().to_string();
    insert_pick(pool, kind, &day, false).await?;

    let latest = format!(
        "SELECT item_id FROM daily_picks
         WHERE day = ?1 AND kind = ?2 AND item_id IN ({})
         ORDER BY id DESC LIMIT 1",
        kind.candidates(),
    );
    let id: Option<i64> = sqlx::query_scalar(&latest)
        .bind(&day)
        .bind(kind.as_str())
        .fetch_optional(pool)
        .await?;

    match id {
        Some(id) => resources::get(pool, id).await,
        None     => Ok(None),
    }
}

/// Replace today's pick for `kind` with a new random one.
pub async fn reroll(pool: &SqlitePool, kind: Kind) -> Result<()> {
    insert_pick(pool, kind, &today().to_string(), true).await
}

/// Record a random pick for `kind` on `day`. Unless `force` is set, does
/// nothing when `day` already has a pick whose item still exists. The check
/// and the insert are one statement, so concurrent requests can't both pick.
///
/// Items among the last `count / 2` picks are skipped, so nothing repeats
/// until at least half the collection has been shown.
async fn insert_pick(pool: &SqlitePool, kind: Kind, day: &str, force: bool) -> Result<()> {
    let candidates = kind.candidates();
    let unless_picked = if force {
        String::new()
    } else {
        format!(
            "AND NOT EXISTS (
                 SELECT 1 FROM daily_picks
                 WHERE day = ?1 AND kind = ?2 AND item_id IN ({candidates})
             )"
        )
    };
    let sql = format!(
        "INSERT INTO daily_picks (day, kind, item_id)
         SELECT ?1, ?2, id FROM ({candidates})
         WHERE id NOT IN (
             SELECT item_id FROM daily_picks WHERE kind = ?2
             ORDER BY id DESC
             LIMIT (SELECT COUNT(*) FROM ({candidates})) / 2
         )
         {unless_picked}
         ORDER BY RANDOM() LIMIT 1"
    );
    sqlx::query(&sql).bind(day).bind(kind.as_str()).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(utc: &str) -> String {
        day_at(utc.parse().unwrap()).to_string()
    }

    #[test]
    fn rolls_over_at_five_paris_time() {
        // Winter (UTC+1): 05:00 Paris is 04:00 UTC.
        assert_eq!(day("2026-01-15T03:59:59Z"), "2026-01-14");
        assert_eq!(day("2026-01-15T04:00:00Z"), "2026-01-15");
        // Summer (UTC+2): 05:00 Paris is 03:00 UTC.
        assert_eq!(day("2026-07-15T02:59:59Z"), "2026-07-14");
        assert_eq!(day("2026-07-15T03:00:00Z"), "2026-07-15");
    }

    #[test]
    fn rollover_stays_at_five_on_dst_change_days() {
        // Clocks go forward on 2026-03-29 and back on 2026-10-25, both before 05:00.
        assert_eq!(day("2026-03-29T02:59:59Z"), "2026-03-28");
        assert_eq!(day("2026-03-29T03:00:00Z"), "2026-03-29");
        assert_eq!(day("2026-10-25T03:59:59Z"), "2026-10-24");
        assert_eq!(day("2026-10-25T04:00:00Z"), "2026-10-25");
    }
}
