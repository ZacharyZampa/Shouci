//! Preferences that every frontend shares (which dictionaries are enabled).
//! Preferences that belong to one app (its hotkey) stay in that app.

use rusqlite::{Connection, OptionalExtension, params};
use vocab_core::Result;

/// # Errors
///
/// Returns an error if the query fails.
pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

/// # Errors
///
/// Returns an error if the write fails.
pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{get_setting, set_setting};
    use crate::open_in_memory;

    #[test]
    fn settings_round_trip_and_overwrite() {
        let conn = open_in_memory().unwrap();
        assert_eq!(get_setting(&conn, "k").unwrap(), None);
        set_setting(&conn, "k", "a").unwrap();
        set_setting(&conn, "k", "b").unwrap();
        assert_eq!(get_setting(&conn, "k").unwrap().as_deref(), Some("b"));
    }
}
