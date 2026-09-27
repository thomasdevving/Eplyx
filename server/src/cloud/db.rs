//! Postgres pool and embedded, ordered migrations applied at startup under an
//! advisory lock, so several instances can start at once safely.
use anyhow::{Context, Result};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use std::str::FromStr;
use tokio_postgres::NoTls;

const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../../migrations/0001_identity.sql")),
    (
        2,
        include_str!("../../migrations/0002_assignment_actor.sql"),
    ),
];

/// Identity storage uses an operator-provided private transport. No TLS layer
/// is added here; use a trusted private connection, never a public database URL.
pub fn connect(url: &str) -> Result<Pool> {
    let config = tokio_postgres::Config::from_str(url)
        .map_err(|_| anyhow::anyhow!("invalid EPLYX_DATABASE_URL"))?;
    let manager = Manager::from_config(
        config,
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        },
    );
    Pool::builder(manager)
        .max_size(16)
        .build()
        .context("could not build the database pool")
}

pub async fn migrate(pool: &Pool) -> Result<()> {
    let mut client = pool
        .get()
        .await
        .context("could not connect to the database")?;
    let tx = client.transaction().await?;
    tx.execute("SELECT pg_advisory_xact_lock(1829)", &[])
        .await?;
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS eplyx_identity_migrations (version INT PRIMARY KEY, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())",
    )
    .await?;
    for (version, sql) in MIGRATIONS {
        let applied = tx
            .query_opt(
                "SELECT 1 FROM eplyx_identity_migrations WHERE version = $1",
                &[version],
            )
            .await?
            .is_some();
        if !applied {
            tx.batch_execute(sql)
                .await
                .with_context(|| format!("migration {version} failed"))?;
            tx.execute(
                "INSERT INTO eplyx_identity_migrations (version) VALUES ($1)",
                &[version],
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}
