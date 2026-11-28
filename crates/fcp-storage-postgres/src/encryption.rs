//! Encryption at rest for the PostgreSQL backend: key-ring persistence
//! (`data_encryption_keys`), seal/open helpers, rotation and bulk re-seal.
//!
//! See `fcp-crypto` for the cipher and provider model. This module owns the
//! part that touches the database: which DEKs exist, which one is active,
//! and which columns are sealed.

use anyhow::{bail, Context, Result};
use fcp_core::encryption::{EncryptionStatus, ReencryptReport, RewrapReport};
use fcp_crypto::{CryptoError, FieldCipher, Kek};
use sqlx::{Pool, Postgres, Row};
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

/// A sealed column: `table.column`, addressed by `id`. `json_field` marks a
/// key inside a JSONB column instead of a plain text column.
pub(crate) struct SealedColumn {
    pub table: &'static str,
    pub column: &'static str,
    pub json_field: Option<&'static str>,
}

impl SealedColumn {
    pub fn name(&self) -> String {
        match self.json_field {
            Some(f) => format!("{}.{}->>{}", self.table, self.column, f),
            None => format!("{}.{}", self.table, self.column),
        }
    }
}

/// Every column the backend seals. Keep in sync with the seal/open call
/// sites in `lib.rs`; the re-encrypt pass walks this list.
pub(crate) const SEALED_COLUMNS: &[SealedColumn] = &[
    SealedColumn {
        table: "users",
        column: "digest_password",
        json_field: None,
    },
    SealedColumn {
        table: "users",
        column: "sip_ha1_hash",
        json_field: None,
    },
    SealedColumn {
        table: "users",
        column: "sip_provisioning_password",
        json_field: None,
    },
    SealedColumn {
        table: "identity_providers",
        column: "client_secret",
        json_field: None,
    },
    SealedColumn {
        table: "user_totp",
        column: "secret",
        json_field: None,
    },
    SealedColumn {
        table: "provisioning_keys",
        column: "key_sealed",
        json_field: None,
    },
    SealedColumn {
        table: "redirection_accounts",
        column: "secret_sealed",
        json_field: None,
    },
    SealedColumn {
        table: "webinar_rtmp_destinations",
        column: "key_sealed",
        json_field: None,
    },
    SealedColumn {
        table: "devices",
        column: "sip_ha1_hash",
        json_field: None,
    },
    SealedColumn {
        table: "devices",
        column: "sip_provisioning_password",
        json_field: None,
    },
    SealedColumn {
        table: "trunks",
        column: "auth_password",
        json_field: None,
    },
    SealedColumn {
        table: "conference_meetings",
        column: "guest_pin",
        json_field: None,
    },
    SealedColumn {
        table: "conference_meetings",
        column: "host_pin",
        json_field: None,
    },
    SealedColumn {
        table: "conference_meetings",
        column: "panelist_pin",
        json_field: None,
    },
    SealedColumn {
        table: "webhook_endpoints",
        column: "endpoint_json",
        json_field: Some("secret"),
    },
    SealedColumn {
        table: "webhook_endpoints",
        column: "endpoint_json",
        json_field: Some("previous_secret"),
    },
    SealedColumn {
        table: "subject_pseudonym_keys",
        column: "secret",
        json_field: None,
    },
    // A tenant's own TTS vendor keys (docs/TENANT_SPEECH.md S3).
    SealedColumn {
        table: "speech_accounts",
        column: "secret",
        json_field: None,
    },
    // Backup destinations' secrets (docs/SCHEDULED_BACKUPS.md B1).
    SealedColumn {
        table: "backup_destinations",
        column: "password",
        json_field: None,
    },
    SealedColumn {
        table: "backup_destinations",
        column: "private_key",
        json_field: None,
    },
    SealedColumn {
        table: "backup_destinations",
        column: "secret_access_key",
        json_field: None,
    },
];

/// Live encryption state for one backend instance.
pub(crate) struct EncryptionState {
    kek: Arc<dyn Kek>,
    /// The KEK the deployment is moving from, while it moves: it unwraps
    /// the keys it wrapped until [`EncryptionState::rewrap`] moves them.
    previous: Option<Arc<dyn Kek>>,
    provider: &'static str,
    cipher: RwLock<FieldCipher>,
}

impl std::fmt::Debug for EncryptionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptionState")
            .field("provider", &self.provider)
            .field("kek_id", &self.kek.id())
            .finish()
    }
}

impl EncryptionState {
    /// Load the key ring, creating the first DEK when the table is empty.
    pub async fn load(
        pool: &Pool<Postgres>,
        kek: Arc<dyn Kek>,
        previous: Option<Arc<dyn Kek>>,
        provider: &'static str,
    ) -> Result<Self> {
        let cipher = match load_ring(pool, kek.as_ref(), previous.as_deref()).await? {
            Some(cipher) => cipher,
            None => {
                create_first_dek(pool, kek.as_ref()).await?;
                load_ring(pool, kek.as_ref(), previous.as_deref())
                    .await?
                    .context("data encryption key ring empty after creating the first key")?
            }
        };
        info!(
            provider,
            kek_id = kek.id(),
            active_key = cipher.active_key_id(),
            keys = cipher.key_ids().len(),
            "Encryption at rest enabled"
        );
        Ok(Self {
            kek,
            previous,
            provider,
            cipher: RwLock::new(cipher),
        })
    }

    pub fn seal(&self, plaintext: &str) -> Result<String> {
        Ok(self.cipher.read().expect("cipher lock").seal(plaintext)?)
    }

    /// Open a stored value; a DEK this process has not seen yet (another
    /// node rotated) triggers one key-ring refresh before failing.
    pub async fn open(&self, pool: &Pool<Postgres>, stored: &str) -> Result<String> {
        let first = self.cipher.read().expect("cipher lock").open(stored);
        match first {
            Ok(opened) => Ok(opened.into_value()),
            Err(CryptoError::UnknownKey(id)) => {
                warn!(key_id = %id, "Sealed value uses an unknown DEK; refreshing key ring");
                self.refresh(pool).await?;
                Ok(self
                    .cipher
                    .read()
                    .expect("cipher lock")
                    .open(stored)?
                    .into_value())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Re-read the key ring from the database.
    pub async fn refresh(&self, pool: &Pool<Postgres>) -> Result<()> {
        let cipher = load_ring(pool, self.kek.as_ref(), self.previous.as_deref())
            .await?
            .context("data encryption key ring is empty")?;
        *self.cipher.write().expect("cipher lock") = cipher;
        Ok(())
    }

    /// Create a new active DEK and pick it up.
    pub async fn rotate(&self, pool: &Pool<Postgres>) -> Result<EncryptionStatus> {
        let id = create_dek(pool, self.kek.as_ref(), true).await?;
        self.refresh(pool).await?;
        info!(key_id = %id, "Data encryption key rotated");
        Ok(self.status())
    }

    pub fn status(&self) -> EncryptionStatus {
        let cipher = self.cipher.read().expect("cipher lock");
        EncryptionStatus {
            enabled: true,
            provider: self.provider.to_string(),
            kek_id: Some(self.kek.id().to_string()),
            active_key_id: Some(cipher.active_key_id().to_string()),
            key_ids: cipher.key_ids(),
            sealed_columns: SEALED_COLUMNS.iter().map(|c| c.name()).collect(),
        }
    }

    /// Move every key the previous KEK wrapped to the current one: unwrapped
    /// with the previous, wrapped with the current, the row's KEK id with
    /// it, one key at a time. Values sealed with those keys are untouched —
    /// a key's material does not change, only what wraps it.
    pub async fn rewrap(&self, pool: &Pool<Postgres>) -> Result<RewrapReport> {
        let rows = sqlx::query("SELECT id, kek_id, wrapped_key FROM data_encryption_keys")
            .fetch_all(pool)
            .await?;
        let mut report = RewrapReport::default();
        for row in rows {
            let id: String = row.get("id");
            let kek_id: String = row.get("kek_id");
            if kek_id == self.kek.id() {
                report.already_current += 1;
                continue;
            }
            let Some(previous) = self.previous.as_ref().filter(|p| p.id() == kek_id) else {
                warn!(key_id = %id, wrapped_by = %kek_id, "A data encryption key no KEK here can unwrap");
                report.unreadable.push(id);
                continue;
            };
            let wrapped: Vec<u8> = row.get("wrapped_key");
            let dek = previous
                .unwrap_dek(&wrapped)
                .await
                .with_context(|| format!("unwrapping data encryption key {id}"))?;
            let rewrapped = self
                .kek
                .wrap_dek(&dek)
                .await
                .with_context(|| format!("wrapping data encryption key {id}"))?;
            sqlx::query(
                "UPDATE data_encryption_keys SET wrapped_key = $1, kek_id = $2
                  WHERE id = $3 AND kek_id = $4",
            )
            .bind(&rewrapped)
            .bind(self.kek.id())
            .bind(&id)
            .bind(&kek_id)
            .execute(pool)
            .await?;
            info!(key_id = %id, from = %kek_id, to = self.kek.id(), "Data encryption key rewrapped");
            report.rewrapped += 1;
        }
        self.refresh(pool).await?;
        Ok(report)
    }

    /// Re-seal every stored secret that is plaintext or uses a non-active DEK.
    pub async fn reencrypt(&self, pool: &Pool<Postgres>) -> Result<ReencryptReport> {
        let mut report = ReencryptReport::default();
        for col in SEALED_COLUMNS {
            let select = match col.json_field {
                Some(field) => format!(
                    "SELECT id, {col}->>'{field}' AS value FROM {table} WHERE {col}->>'{field}' IS NOT NULL",
                    col = col.column,
                    table = col.table,
                ),
                None => format!(
                    "SELECT id, {col} AS value FROM {table} WHERE {col} IS NOT NULL",
                    col = col.column,
                    table = col.table,
                ),
            };
            let update = match col.json_field {
                Some(field) => format!(
                    "UPDATE {table} SET {col} = jsonb_set({col}, '{{{field}}}', to_jsonb($1::text)) WHERE id = $2",
                    col = col.column,
                    table = col.table,
                ),
                None => format!(
                    "UPDATE {table} SET {col} = $1 WHERE id = $2",
                    col = col.column,
                    table = col.table,
                ),
            };

            let rows = sqlx::query(&select).fetch_all(pool).await?;
            let mut rewritten = 0u64;
            for row in rows {
                let id: String = row.get("id");
                let value: String = row.get("value");
                let resealed = self
                    .cipher
                    .read()
                    .expect("cipher lock")
                    .reseal_if_stale(&value);
                match resealed {
                    Ok(Some(new_value)) => {
                        sqlx::query(&update)
                            .bind(new_value)
                            .bind(&id)
                            .execute(pool)
                            .await?;
                        rewritten += 1;
                    }
                    Ok(None) => report.already_current += 1,
                    Err(e) => bail!("cannot re-seal {} id={id}: {e}", col.name()),
                }
            }
            report.rewritten += rewritten;
            report.by_column.push((col.name(), rewritten));
        }
        info!(
            rewritten = report.rewritten,
            already_current = report.already_current,
            "Re-encryption pass complete"
        );
        Ok(report)
    }
}

/// Read every DEK, unwrap with `kek` (or, for a key it wrapped, with
/// `previous`), and build the cipher. `None` when the table is empty.
async fn load_ring(
    pool: &Pool<Postgres>,
    kek: &dyn Kek,
    previous: Option<&dyn Kek>,
) -> Result<Option<FieldCipher>> {
    let rows = sqlx::query(
        "SELECT id, kek_id, wrapped_key, active FROM data_encryption_keys ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }

    let mut keys = Vec::with_capacity(rows.len());
    let mut active: Option<String> = None;
    for row in rows {
        let id: String = row.get("id");
        let kek_id: String = row.get("kek_id");
        let wrapped: Vec<u8> = row.get("wrapped_key");
        let is_active: bool = row.get("active");

        // A key the previous KEK wrapped, while the deployment moves.
        let unwrapper = match previous {
            Some(previous) if kek_id == previous.id() => previous,
            _ => kek,
        };
        if kek_id != unwrapper.id() {
            if is_active {
                bail!(
                    "active data encryption key {id} was wrapped by KEK {kek_id} but this process \
                     is configured with KEK {} — the master key or KMS key does not match the database",
                    kek.id()
                );
            }
            warn!(
                key_id = %id,
                wrapped_by = %kek_id,
                "Skipping data encryption key wrapped by a different KEK; values sealed with it cannot be read"
            );
            continue;
        }

        let dek = unwrapper
            .unwrap_dek(&wrapped)
            .await
            .with_context(|| format!("unwrapping data encryption key {id}"))?;
        keys.push((id.clone(), dek));
        if is_active {
            active = Some(id);
        }
    }

    let active = active.context("no active data encryption key in data_encryption_keys")?;
    Ok(Some(FieldCipher::new(keys, &active)?))
}

/// The advisory lock the first key is made under: one transaction at a
/// time, so the services of a stack starting together on an empty
/// database make one key between them.
const FIRST_KEY_LOCK: i64 = 0x6663_705f_6b65_7973; // "fcp_keys"

/// Make the first DEK of an empty ring, unless another process made it
/// first. Every service of a stack opens the database at once on first
/// boot; without the lock each found the ring empty, each inserted, the
/// unique index on the active key refused all but one, and the losers
/// exited to be restarted (found by the nightly stack,
/// `docs/NIGHTLY_STACK.md` §4).
async fn create_first_dek(pool: &Pool<Postgres>, kek: &dyn Kek) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(FIRST_KEY_LOCK)
        .execute(&mut *tx)
        .await
        .context("taking the first-key lock")?;
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM data_encryption_keys")
        .fetch_one(&mut *tx)
        .await?;
    if existing > 0 {
        // Another service made it while this one waited for the lock.
        tx.rollback().await?;
        return Ok(());
    }
    let (_dek, wrapped) = kek.generate_dek().await?;
    let id = format!("dek-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    sqlx::query(
        "INSERT INTO data_encryption_keys (id, kek_id, wrapped_key, active, created_at)
         VALUES ($1, $2, $3, TRUE, NOW())",
    )
    .bind(&id)
    .bind(kek.id())
    .bind(&wrapped)
    .execute(&mut *tx)
    .await
    .context("inserting the first data encryption key")?;
    tx.commit().await?;
    info!(key_id = %id, "Created the first data encryption key");
    Ok(())
}

/// Generate, wrap and insert a DEK. With `activate`, the previous active key
/// is demoted in the same transaction. Returns the new key id.
async fn create_dek(pool: &Pool<Postgres>, kek: &dyn Kek, activate: bool) -> Result<String> {
    let (_dek, wrapped) = kek.generate_dek().await?;
    let id = format!("dek-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);

    let mut tx = pool.begin().await?;
    if activate {
        sqlx::query("UPDATE data_encryption_keys SET active = FALSE WHERE active")
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "INSERT INTO data_encryption_keys (id, kek_id, wrapped_key, active, created_at)
         VALUES ($1, $2, $3, $4, NOW())",
    )
    .bind(&id)
    .bind(kek.id())
    .bind(&wrapped)
    .bind(activate)
    .execute(&mut *tx)
    .await
    .context("inserting data encryption key")?;
    tx.commit().await?;
    Ok(id)
}
