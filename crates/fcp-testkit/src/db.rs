//! Throwaway PostgreSQL databases for integration and smoke tests.

use sqlx::{Connection, PgConnection};
use uuid::Uuid;

/// Environment variable naming an admin connection string
/// (e.g. `postgresql://postgres:postgres@localhost:5432/postgres`).
pub const DATABASE_URL_ENV: &str = "FCP_TEST_DATABASE_URL";

/// A freshly created, empty database that the test owns.
///
/// The name is prefixed `fcp_test_` so databases left behind by a panicking
/// test are easy to find and drop. Call [`TestDatabase::drop_db`] at the end
/// of a passing test; nothing runs automatically on drop because dropping a
/// database is async.
pub struct TestDatabase {
    admin_url: String,
    name: String,
    url: String,
}

impl TestDatabase {
    /// Create a new database, or return `None` when `FCP_TEST_DATABASE_URL`
    /// is unset so the calling test can skip.
    pub async fn create() -> Option<Self> {
        let admin_url = std::env::var(DATABASE_URL_ENV).ok()?;
        let name = format!("fcp_test_{}", Uuid::new_v4().simple());

        let mut admin = PgConnection::connect(&admin_url)
            .await
            .expect("connect to admin database");
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&mut admin)
            .await
            .expect("create test database");
        admin.close().await.ok();

        let url = swap_database(&admin_url, &name);
        Some(Self {
            admin_url,
            name,
            url,
        })
    }

    /// Connection string for the new database.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Database name (`fcp_test_<hex>`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Drop the database, disconnecting any lingering sessions first.
    pub async fn drop_db(self) {
        let mut admin = PgConnection::connect(&self.admin_url)
            .await
            .expect("connect to admin database");
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.name))
            .execute(&mut admin)
            .await
            .expect("drop test database");
        admin.close().await.ok();
    }
}

/// Swap the database name in a connection string, keeping any query
/// parameters (`?sslmode=…`).
pub fn swap_database(admin_url: &str, database: &str) -> String {
    let (base, query) = match admin_url.split_once('?') {
        Some((b, q)) => (b, format!("?{q}")),
        None => (admin_url, String::new()),
    };
    let (head, _) = base
        .rsplit_once('/')
        .expect("database URL must contain a database path");
    format!("{head}/{database}{query}")
}

#[cfg(test)]
mod tests {
    use super::swap_database;

    #[test]
    fn swaps_the_path_and_keeps_query() {
        assert_eq!(
            swap_database("postgresql://u:p@h:5432/postgres?sslmode=disable", "x"),
            "postgresql://u:p@h:5432/x?sslmode=disable"
        );
        assert_eq!(
            swap_database("postgresql://u@h/postgres", "fcp_test_1"),
            "postgresql://u@h/fcp_test_1"
        );
    }
}
