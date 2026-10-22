use crate::config::Config;
use crate::error::Result;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

/// Open (creating if needed) the database in the data folder and apply any
/// pending migrations.
pub async fn connect(config: &Config) -> Result<SqlitePool> {
    std::fs::create_dir_all(&config.data_dir)?;
    let options = SqliteConnectOptions::new()
        .filename(config.database_path())
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new().connect_with(options).await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}
