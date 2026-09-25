//! An old binary against a newer schema refuses to start, and says which
//! migration it does not carry; a migration edited after it shipped is
//! refused the same way (`docs/BACKUP_AND_RESTORE.md` §5 rules 3 and 4,
//! `docs/BACKUP_RESTORE_UPGRADE.md` phase 5). Gated on
//! `FCP_TEST_DATABASE_URL`; skipped with a notice when unset.

use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use sqlx::{Connection, PgConnection};

fn swap_database(url: &str, name: &str) -> String {
    let (base, _) = url.rsplit_once('/').expect("a database in the url");
    format!("{base}/{name}")
}

fn config(url: &str) -> PostgresConfig {
    PostgresConfig {
        connection_string: url.to_string(),
        max_connections: 2,
        ..Default::default()
    }
}

#[tokio::test]
async fn a_binary_older_than_the_schema_refuses_to_start_naming_the_migration() {
    let Ok(admin_url) = std::env::var("FCP_TEST_DATABASE_URL") else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the upgrade refusal test");
        return;
    };
    let name = format!("fcp_test_{}", uuid::Uuid::new_v4().simple());
    let mut admin = PgConnection::connect(&admin_url).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&mut admin)
        .await
        .unwrap();
    let url = swap_database(&admin_url, &name);

    // This release opens the database and leaves its schema behind, as
    // the first new node of an upgrade does.
    drop(
        PostgresStorage::new(config(&url))
            .await
            .expect("this release opens an empty database"),
    );

    // A later release applied a migration this binary does not carry.
    let mut conn = PgConnection::connect(&url).await.unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time) \
         VALUES (999999, 'a later release', now(), true, '\\x00'::bytea, 0)",
    )
    .execute(&mut conn)
    .await
    .unwrap();
    let refused = PostgresStorage::new(config(&url))
        .await
        .err()
        .expect("an older binary refuses a newer schema");
    let text = format!("{refused:#}");
    assert!(
        text.contains("999999") && text.to_lowercase().contains("missing"),
        "the refusal names the migration it does not carry: {text}"
    );

    // Rolled back to this release's schema, but a shipped migration was
    // edited: every node refuses that too.
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 999999")
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\x00'::bytea WHERE version = 1")
        .execute(&mut conn)
        .await
        .unwrap();
    let refused = PostgresStorage::new(config(&url))
        .await
        .err()
        .expect("an edited migration is refused");
    let text = format!("{refused:#}").to_lowercase();
    assert!(
        text.contains("migration 1 ") && (text.contains("modified") || text.contains("mismatch")),
        "the refusal names the edited migration: {text}"
    );
    conn.close().await.ok();

    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&mut admin)
        .await
        .unwrap();
}
