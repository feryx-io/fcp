//! Attributing deletions (`docs/COMPLIANCE_FOLLOWUPS.md` §7). Migration 062's
//! triggers write the deletion log from the transaction that deletes; this
//! opens that transaction with the current task's deleter
//! (`fcp_core::compliance::deleting_as`) as its cause and actor.

use sqlx::{Pool, Postgres, Transaction};

/// A transaction whose deletions the log attributes to the current deleter,
/// or to nobody (`unattributed`) outside a [`deleting_as`] scope.
///
/// [`deleting_as`]: fcp_core::compliance::deleting_as
pub(crate) async fn attributed(
    pool: &Pool<Postgres>,
) -> sqlx::Result<Transaction<'static, Postgres>> {
    let mut tx = pool.begin().await?;
    if let Some(deleter) = fcp_core::compliance::current_deleter() {
        sqlx::query(
            "SELECT set_config('fcp.deletion_cause', $1, true),
                    set_config('fcp.deletion_actor', $2, true),
                    set_config('fcp.deletion_request', $3, true)",
        )
        .bind(deleter.cause.as_str())
        .bind(&deleter.actor)
        .bind(deleter.request_id.as_deref().unwrap_or(""))
        .execute(&mut *tx)
        .await?;
    }
    Ok(tx)
}

/// Run one deleting statement in an attributed transaction. A delete a
/// legal hold refuses (migration 063's triggers) comes back as a
/// [`LegalHoldViolation`](fcp_core::legal_hold::LegalHoldViolation).
pub(crate) async fn execute(
    pool: &Pool<Postgres>,
    query: sqlx::query::Query<'_, Postgres, sqlx::postgres::PgArguments>,
) -> anyhow::Result<u64> {
    let mut tx = attributed(pool).await?;
    let affected = match query.execute(&mut *tx).await {
        Ok(done) => done.rows_affected(),
        Err(e) => return Err(held(e)),
    };
    tx.commit().await.map_err(held)?;
    Ok(affected)
}

/// A database error, or the legal hold it names.
pub(crate) fn held(e: sqlx::Error) -> anyhow::Error {
    if let Some(db) = e.as_database_error() {
        if db.code().as_deref() == Some("HLD01") {
            let hold_id = db
                .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
                .and_then(|pg| pg.detail())
                .unwrap_or_default()
                .to_string();
            return anyhow::Error::new(fcp_core::legal_hold::LegalHoldViolation { hold_id });
        }
    }
    e.into()
}

/// Whether `tenant` holds anything: a retention delete adds the hold
/// filter only then.
pub(crate) async fn tenant_has_holds(pool: &Pool<Postgres>, tenant: &str) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar("SELECT fcp_tenant_has_holds($1)")
        .bind(tenant)
        .fetch_one(pool)
        .await?)
}
