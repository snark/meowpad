use anyhow::Result;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};

pub fn migrate(mut conn: Connection) -> Result<()> {
    let migrations = Migrations::new(vec![
        M::up(include_str!("../migrations/001.sql")),
        M::up(include_str!("../migrations/002.sql")),
    ]);
    migrations.to_latest(&mut conn)?;
    Ok(())
}
