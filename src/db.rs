// Phase 1 scaffold: sqlite schema + state enum. Consumed starting Phase 2.
#![allow(dead_code)]

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Current schema version. Bump and add a migration when changing the schema.
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
            .ok_or_else(|| anyhow::anyhow!("no $XDG_DATA_HOME"))?;
        Ok(dir)
    }
}
