use rusqlite::Connection;

use crate::StoreError;

/// Bring a database up to the newest schema. Each entry in `steps` takes it
/// from the version of its index to the next; the version is SQLite's own
/// `user_version`. A step and its version change commit together, so a crash
/// part-way through leaves the database at a version it really is, and two
/// copies of Brolga opening it at once run each step once between them.
pub(crate) fn migrate(connection: &mut Connection, steps: &[&str]) -> Result<(), StoreError> {
    let known = u32::try_from(steps.len()).unwrap_or(u32::MAX);
    let found: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    // Never open, and so never damage, a database from a newer Brolga.
    if found > known {
        return Err(StoreError::Newer { found, known });
    }

    for (version, step) in (1..=known).zip(steps).skip(found as usize) {
        // Another copy of Brolga may be opening the same database at the
        // same moment. The step is begun as a write, which only one of
        // them can be in at a time, and the version read again inside
        // it: whoever comes second finds the step done and leaves it.
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now: u32 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if now >= version {
            continue;
        }
        transaction.execute_batch(step)?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }

    // And if the other copy was a newer Brolga, which has taken it further.
    let found: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > known {
        return Err(StoreError::Newer { found, known });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = "CREATE TABLE a (x INTEGER);";
    const V2: &str = "ALTER TABLE a ADD COLUMN y INTEGER; INSERT INTO a (x, y) VALUES (1, 2);";

    fn version(connection: &Connection) -> u32 {
        connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_new_database_gets_every_step() {
        let mut connection = Connection::open_in_memory().unwrap();

        migrate(&mut connection, &[V1, V2]).unwrap();

        assert_eq!(version(&connection), 2);
        assert_eq!(
            connection
                .query_row("SELECT y FROM a", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn an_older_database_gets_only_the_steps_it_lacks() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection, &[V1]).unwrap();

        migrate(&mut connection, &[V1, V2]).unwrap();
        // Running again changes nothing: the insert in V2 happened once.
        migrate(&mut connection, &[V1, V2]).unwrap();

        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM a", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn a_step_that_fails_leaves_the_database_where_it_was() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection, &[V1]).unwrap();

        let result = migrate(
            &mut connection,
            &[V1, "ALTER TABLE a ADD COLUMN y INTEGER; this is not sql;"],
        );

        assert!(result.is_err());
        assert_eq!(version(&connection), 1);
        // The half of the step that did run was rolled back with it.
        assert!(connection.prepare("SELECT y FROM a").is_err());
    }

    #[test]
    fn a_database_from_a_newer_version_is_not_touched() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection, &[V1, V2]).unwrap();

        let result = migrate(&mut connection, &[V1]);

        assert!(matches!(
            result,
            Err(StoreError::Newer { found: 2, known: 1 })
        ));
    }

    #[test]
    fn two_copies_opening_the_same_database_at_once_each_step_is_run_by_one_of_them() {
        let folder = std::env::temp_dir().join(format!("brolga-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();

        for round in 0..20 {
            let path = folder.join(format!("{round}.db"));
            let _ = std::fs::remove_file(&path);
            migrate(&mut Connection::open(&path).unwrap(), &[V1]).unwrap();

            // Each sees version 1 and has a step to run that cannot be run twice.
            let together = std::sync::Arc::new(std::sync::Barrier::new(6));
            let all: Vec<_> = (0..6)
                .map(|_| {
                    let (path, together) = (path.clone(), together.clone());
                    std::thread::spawn(move || {
                        let mut connection = Connection::open(&path).unwrap();
                        together.wait();
                        migrate(&mut connection, &[V1, V2])
                    })
                })
                .collect();
            for each in all {
                each.join().unwrap().unwrap();
            }

            let connection = Connection::open(&path).unwrap();
            assert_eq!(version(&connection), 2);
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM a", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
        let _ = std::fs::remove_dir_all(&folder);
    }
}
