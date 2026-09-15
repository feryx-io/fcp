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
                    set_config('fcp.deletion_actor', $2, true)",
        )
        .bind(deleter.cause.as_str())
        .bind(&deleter.actor)
        .execute(&mut *tx)
        .await?;
    }
    Ok(tx)
}

/// Run one deleting statement in an attributed transaction.
pub(crate) async fn execute(
    pool: &Pool<Postgres>,
    query: sqlx::query::Query<'_, Postgres, sqlx::postgres::PgArguments>,
) -> sqlx::Result<u64> {
    let mut tx = attributed(pool).await?;
    let affected = query.execute(&mut *tx).await?.rows_affected();
    tx.commit().await?;
    Ok(affected)
}
