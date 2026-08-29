use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{path::Path, time::Duration};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("could not create the database directory")]
    Directory(#[from] std::io::Error),
    #[error("could not connect to the database")]
    Database(#[from] sqlx::Error),
    #[error("database schema version {0} is newer than this server supports")]
    SchemaVersion(i64),
}

pub async fn connect_database(path: &Path) -> Result<SqlitePool, StorageError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &SqlitePool) -> Result<(), StorageError> {
    let version = sqlx::query_scalar::<_, i64>("PRAGMA user_version")
        .fetch_one(pool)
        .await?;
    match version {
        0 => {
            let mut transaction = pool.begin().await?;
            sqlx::raw_sql(include_str!("../migrations/0001_initial.sql"))
                .execute(&mut *transaction)
                .await?;
            sqlx::query("PRAGMA user_version = 1")
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            Ok(())
        }
        1 => Ok(()),
        other => Err(StorageError::SchemaVersion(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;
    use tempfile::TempDir;

    #[tokio::test]
    async fn migration_enables_wal_and_persists_operations() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("sync.db");
        let pool = connect_database(&path).await.unwrap();
        let journal_mode: String = sqlx::query("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
        assert_eq!(journal_mode, "wal");
        sqlx::query("INSERT INTO devices(device_id, last_seen_at, last_cursor) VALUES (?, 1, 0)")
            .bind("ios-phone")
            .execute(&pool)
            .await
            .unwrap();
        drop(pool);

        let reopened = connect_database(&path).await.unwrap();
        let count: i64 = sqlx::query("SELECT COUNT(*) FROM devices")
            .fetch_one(&reopened)
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 1);
    }
}
