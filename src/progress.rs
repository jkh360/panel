use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::detection_cache;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Position {
    pub page: usize,
    pub region: usize,
}

pub struct ProgressStore {
    connection: Connection,
}

impl ProgressStore {
    pub fn open() -> Result<Self> {
        let path = database_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&path)
            .with_context(|| format!("failed to open progress database at {}", path.display()))?;
        initialize(&connection)?;
        Ok(Self { connection })
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        initialize(&connection)?;
        Ok(Self { connection })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn load(&self, path: &Path) -> Result<Option<Position>> {
        let path = document_key(path)?;
        let mut statement = self
            .connection
            .prepare("SELECT page, region FROM progress WHERE path = ?1")?;
        let mut rows = statement.query([path])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(Position {
            page: row.get::<_, i64>(0)?.try_into().unwrap_or(0),
            region: row.get::<_, i64>(1)?.try_into().unwrap_or(0),
        }))
    }

    /// The most recently visited book among the volumes currently displayed.
    pub fn most_recent(&self, books: &[PathBuf]) -> Result<Option<(PathBuf, Position)>> {
        let mut statement = self
            .connection
            .prepare("SELECT page, region, updated_at FROM progress WHERE path = ?1")?;
        let mut latest = None;
        for book in books {
            let path = document_key(book)?;
            let saved: Option<(i64, i64, i64)> = statement
                .query_row([path], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .optional()?;
            if let Some((page, region, updated_at)) = saved {
                if latest
                    .as_ref()
                    .is_none_or(|(time, _, _)| updated_at > *time)
                {
                    latest = Some((
                        updated_at,
                        book.clone(),
                        Position {
                            page: page.try_into().unwrap_or(0),
                            region: region.try_into().unwrap_or(0),
                        },
                    ));
                }
            }
        }
        Ok(latest.map(|(_, book, position)| (book, position)))
    }

    pub fn load_zoom(&self, path: &Path) -> Result<f32> {
        let path = document_key(path)?;
        let zoom: Option<f32> = self
            .connection
            .query_row("SELECT zoom FROM progress WHERE path = ?1", [path], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(zoom
            .filter(|value| value.is_finite() && (0.5..=3.0).contains(value))
            .unwrap_or(1.0))
    }

    pub fn save_zoom(&self, path: &Path, zoom: f32) -> Result<()> {
        anyhow::ensure!(
            zoom.is_finite() && (0.5..=3.0).contains(&zoom),
            "invalid zoom"
        );
        let path = document_key(path)?;
        self.connection.execute(
            "UPDATE progress SET zoom = ?2 WHERE path = ?1",
            params![path, zoom],
        )?;
        Ok(())
    }

    pub fn save(&self, path: &Path, position: Position, completed: bool) -> Result<()> {
        let path = document_key(path)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        self.connection.execute(
            "INSERT INTO progress(path, page, region, completed, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
                 page = excluded.page,
                 region = excluded.region,
                 completed = excluded.completed,
                 updated_at = excluded.updated_at",
            params![
                path,
                position.page as i64,
                position.region as i64,
                completed,
                timestamp
            ],
        )?;
        Ok(())
    }
}

fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS progress (
             path TEXT PRIMARY KEY NOT NULL,
             page INTEGER NOT NULL CHECK(page >= 0),
             region INTEGER NOT NULL CHECK(region >= 0),
             completed INTEGER NOT NULL CHECK(completed IN (0, 1)),
             updated_at INTEGER NOT NULL,
             zoom REAL NOT NULL DEFAULT 1.0
         );",
    )?;
    let mut columns = connection.prepare("PRAGMA table_info(progress)")?;
    let has_zoom = columns
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|column| column == "zoom");
    if !has_zoom {
        connection
            .execute_batch("ALTER TABLE progress ADD COLUMN zoom REAL NOT NULL DEFAULT 1.0")?;
    }
    detection_cache::initialize(connection)?;
    Ok(())
}

fn database_path() -> Result<PathBuf> {
    if let Some(state_home) = env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(state_home).join("panel/library.sqlite3"));
    }
    let home = env::var_os("HOME").context("neither XDG_STATE_HOME nor HOME is set")?;
    Ok(PathBuf::from(home).join(".local/state/panel/library.sqlite3"))
}

fn document_key(path: &Path) -> Result<String> {
    Ok(path
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", path.display()))?
        .to_string_lossy()
        .into_owned())
}

#[cfg(test)]
mod tests {
    use super::initialize;

    #[test]
    fn existing_progress_gains_zoom_without_losing_position() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE progress (
                path TEXT PRIMARY KEY, page INTEGER NOT NULL, region INTEGER NOT NULL,
                completed INTEGER NOT NULL, updated_at INTEGER NOT NULL
            );
            INSERT INTO progress VALUES ('book', 42, 2, 0, 1);",
            )
            .unwrap();
        initialize(&connection).unwrap();
        initialize(&connection).unwrap();
        let (page, zoom): (i64, f32) = connection
            .query_row(
                "SELECT page, zoom FROM progress WHERE path = 'book'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(page, 42);
        assert_eq!(zoom, 1.0);
    }
}
