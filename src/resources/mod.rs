pub mod artwork;
pub mod definition;
pub mod quote;
pub mod scripture;

use crate::error::Result;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteRow;

/// A row type that can be loaded by id, e.g. for today's pick.
pub trait Resource: for<'r> sqlx::FromRow<'r, SqliteRow> + Send + Unpin {
    /// `SELECT <columns> FROM <table>`, without a WHERE clause.
    const SELECT: &'static str;
}

pub async fn get<T: Resource>(pool: &SqlitePool, id: i64) -> Result<Option<T>> {
    let sql = format!("{} WHERE id = ?", T::SELECT);
    Ok(sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?)
}
