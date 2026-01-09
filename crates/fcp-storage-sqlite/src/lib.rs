//! SQLite storage backend for FCP.
//!
//! This module provides a production-ready SQLite implementation of the FCP storage traits.
//! It uses sqlx for async database access with connection pooling and migrations.

use anyhow::{Context, Result};
use async_trait::async_trait;
use fcp_core::{
    CallDetailRecord, CallDisposition, CdrQuery, CdrStore, CredentialStore, HangupCause,
    HangupParty, MediaPreferences, Registration, RegistrationStore, TenantId, User,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Pool, Row, Sqlite, SqlitePool};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

/// Configuration for SQLite storage backend.
#[derive(Debug, Clone)]
pub struct SqliteConfig {
    pub database_path: PathBuf,
    pub max_connections: u32,
    pub connection_timeout: Duration,
    pub enable_wal: bool,
    pub create_if_missing: bool,
}

impl SqliteConfig {
    pub fn new<P: Into<PathBuf>>(database_path: P) -> Self {
        Self {
            database_path: database_path.into(),
            max_connections: 5,
            connection_timeout: Duration::from_secs(30),
            enable_wal: true,
            create_if_missing: true,
        }
    }
}

/// SQLite storage backend implementing all FCP storage traits.
#[derive(Clone)]
pub struct SqliteStorage {
    pool: Pool<Sqlite>,
}

impl SqliteStorage {
    pub async fn new(config: SqliteConfig) -> Result<Self> {
        if let Some(parent) = config.database_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut options = SqliteConnectOptions::from_str(
            &format!("sqlite://{}", config.database_path.display()),
        )?
        .create_if_missing(config.create_if_missing);

        if config.enable_wal {
            options = options.journal_mode(SqliteJournalMode::Wal);
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(config.connection_timeout)
            .connect_with(options)
            .await?;

        info!("SQLite storage initialized");
        Self::run_migrations(&pool).await?;

        Ok(Self { pool })
    }

    async fn run_migrations(pool: &SqlitePool) -> Result<()> {
        let migration_sql = include_str!("../migrations/001_initial_schema.sql");
        for statement in migration_sql.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() && !trimmed.starts_with("--") {
                sqlx::query(trimmed).execute(pool).await?;
            }
        }
        info!("SQLite migrations completed");
        Ok(())
    }
}
