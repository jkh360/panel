use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

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
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS progress (
                 path TEXT PRIMARY KEY NOT NULL,
                 page INTEGER NOT NULL CHECK(page >= 0),
                 region INTEGER NOT NULL CHECK(region >= 0),
                 completed INTEGER NOT NULL CHECK(completed IN (0, 1)),
                 updated_at INTEGER NOT NULL
             );",
        )?;
        detection_cache::initialize(&connection)?;
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

    pub fn save(&self, path: &Path, position: Position, completed: bool) -> Result<()> {
        let path = document_key(path)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
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
