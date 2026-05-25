#![allow(dead_code)]

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Bump and add a migration when changing the schema.
const SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Discovered,
    Fetched,
    Verified,
    Signed,
    Published,
    Failed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Discovered => "discovered",
            State::Fetched => "fetched",
            State::Verified => "verified",
            State::Signed => "signed",
            State::Published => "published",
            State::Failed => "failed",
        }
    }
}

impl FromStr for State {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "discovered" => Ok(State::Discovered),
            "fetched" => Ok(State::Fetched),
            "verified" => Ok(State::Verified),
            "signed" => Ok(State::Signed),
            "published" => Ok(State::Published),
            "failed" => Ok(State::Failed),
            other => Err(anyhow!("unknown state: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Build {
    pub run_id: u64,
    pub head_sha: String,
    pub ref_name: String,
    pub state: State,
    pub discovered_at: i64,
    pub updated_at: i64,
    pub error: Option<String>,
    pub release_id: Option<i64>,
    pub release_tag: Option<String>,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("creating data dir {}", data_dir.display()))?;
        let path = data_dir.join("builds.db");
        let conn = Connection::open(&path)
            .with_context(|| format!("opening sqlite db at {}", path.display()))?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;

        let current: i32 = self
            .conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'version'",
                [],
                |row| row.get::<_, String>(0).map(|s| s.parse().unwrap_or(0)),
            )
            .unwrap_or(0);

        if current < 1 {
            self.conn.execute_batch(
                "CREATE TABLE builds (
                    run_id        INTEGER PRIMARY KEY,
                    head_sha      TEXT NOT NULL,
                    ref_name      TEXT NOT NULL,
                    state         TEXT NOT NULL,
                    discovered_at INTEGER NOT NULL,
                    updated_at    INTEGER NOT NULL,
                    error         TEXT,
                    release_id    INTEGER,
                    release_tag   TEXT
                );
                CREATE INDEX builds_state_idx ON builds(state);
                CREATE INDEX builds_ref_idx ON builds(ref_name);",
            )?;
        }

        self.conn.execute(
            "INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('version', ?1)",
            [&SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    pub fn data_dir_for(config_override: Option<&Path>) -> Result<PathBuf> {
        if let Some(p) = config_override {
            return Ok(p.to_path_buf());
        }
        let xdg = xdg::BaseDirectories::with_prefix("navio-signer");
        let dir = xdg
            .get_data_home()
            .ok_or_else(|| anyhow!("no $XDG_DATA_HOME"))?;
        Ok(dir)
    }

    pub fn contains_run(&self, run_id: u64) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM builds WHERE run_id = ?1",
            [run_id as i64],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Insert a freshly-discovered run. No-op if run_id already known.
    /// Returns true if a new row was inserted.
    pub fn insert_discovered(&self, run_id: u64, head_sha: &str, ref_name: &str) -> Result<bool> {
        let now = now_unix();
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO builds
                (run_id, head_sha, ref_name, state, discovered_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                run_id as i64,
                head_sha,
                ref_name,
                State::Discovered.as_str(),
                now,
            ],
        )?;
        Ok(changed > 0)
    }

    /// Update an existing build's state. If `error` is Some, it's stored;
    /// if None, the error column is cleared.
    pub fn set_state(&self, run_id: u64, state: State, error: Option<&str>) -> Result<()> {
        let now = now_unix();
        let changed = self.conn.execute(
            "UPDATE builds
             SET state = ?1, updated_at = ?2, error = ?3
             WHERE run_id = ?4",
            params![state.as_str(), now, error, run_id as i64],
        )?;
        if changed == 0 {
            bail!("no build row for run_id {run_id}");
        }
        Ok(())
    }

    /// Update a build's recorded release identifiers (used by the publisher).
    pub fn set_release(
        &self,
        run_id: u64,
        release_id: Option<i64>,
        release_tag: Option<&str>,
    ) -> Result<()> {
        let now = now_unix();
        self.conn.execute(
            "UPDATE builds
             SET release_id = ?1, release_tag = ?2, updated_at = ?3
             WHERE run_id = ?4",
            params![release_id, release_tag, now, run_id as i64],
        )?;
        Ok(())
    }

    pub fn list_all(&self) -> Result<Vec<Build>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, head_sha, ref_name, state, discovered_at, updated_at,
                    error, release_id, release_tag
             FROM builds
             ORDER BY discovered_at DESC",
        )?;
        let rows = stmt
            .query_map([], row_to_build)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_by_state(&self, state: State) -> Result<Vec<Build>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, head_sha, ref_name, state, discovered_at, updated_at,
                    error, release_id, release_tag
             FROM builds
             WHERE state = ?1
             ORDER BY discovered_at ASC",
        )?;
        let rows = stmt
            .query_map([state.as_str()], row_to_build)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get(&self, run_id: u64) -> Result<Option<Build>> {
        self.conn
            .query_row(
                "SELECT run_id, head_sha, ref_name, state, discovered_at, updated_at,
                        error, release_id, release_tag
                 FROM builds
                 WHERE run_id = ?1",
                [run_id as i64],
                row_to_build,
            )
            .optional()
            .map_err(Into::into)
    }
}

fn row_to_build(row: &rusqlite::Row<'_>) -> rusqlite::Result<Build> {
    let state_str: String = row.get(3)?;
    let state = State::from_str(&state_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(e.to_string())),
        )
    })?;
    Ok(Build {
        run_id: row.get::<_, i64>(0)? as u64,
        head_sha: row.get(1)?,
        ref_name: row.get(2)?,
        state,
        discovered_at: row.get(4)?,
        updated_at: row.get(5)?,
        error: row.get(6)?,
        release_id: row.get(7)?,
        release_tag: row.get(8)?,
    })
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
