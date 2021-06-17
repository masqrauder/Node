// Copyright (c) 2019-2021, MASQ (https://masq.ai) and/or its affiliates. All rights reserved.

use crate::database::connection_wrapper::ConnectionWrapper;
use crate::database::db_initializer::CURRENT_SCHEMA_VERSION;
use crate::sub_lib::logger::Logger;
use masq_lib::utils::ExpectValue;
use rusqlite::{Transaction, NO_PARAMS};
use std::fmt::Debug;

pub trait DbMigrator {
    fn migrate_database(
        &self,
        mismatched_schema: usize,
        target_version: usize,
        conn: Box<dyn ConnectionWrapper>,
    ) -> Result<(), String>;
    fn log_warn(&self, msg: &str);
}

pub struct DbMigratorReal {
    logger: Logger,
}

impl DbMigrator for DbMigratorReal {
    fn migrate_database(
        &self,
        mismatched_schema: usize,
        target_version: usize,
        mut conn: Box<dyn ConnectionWrapper>,
    ) -> Result<(), String> {
        let migrator_config = DBMigratorConfiguration::new();
        let migration_utils = match DBMigrationUtilitiesReal::new(&mut *conn, migrator_config) {
            Err(e) => return Err(e.to_string()),
            Ok(utils) => utils,
        };
        self.make_updates(
            mismatched_schema,
            target_version,
            Box::new(migration_utils),
            Self::list_of_existing_updates(),
        )
    }
    fn log_warn(&self, msg: &str) {
        warning!(self.logger, "{}", msg)
    }
}

impl Default for DbMigratorReal {
    fn default() -> Self {
        Self::new()
    }
}

trait DatabaseMigration: Debug {
    fn migrate(&self, migration_utilities: &dyn DBMigrationUtilities) -> rusqlite::Result<()>;
    fn old_version(&self) -> usize;
}

trait DBMigrationUtilities {
    fn update_schema_version(&self, updated_to: String) -> rusqlite::Result<()>;

    fn execute_upon_transaction(&self, sql_statements: &[&'static str]) -> rusqlite::Result<()>;

    fn commit(&mut self) -> Result<(), String>;
}

struct DBMigrationUtilitiesReal<'a> {
    root_transaction: Option<Transaction<'a>>,
    db_migrator_configuration: DBMigratorConfiguration,
}

impl<'a> DBMigrationUtilitiesReal<'a> {
    fn new<'b: 'a>(
        conn: &'b mut dyn ConnectionWrapper,
        db_migrator_configuration: DBMigratorConfiguration,
    ) -> rusqlite::Result<Self> {
        let new_instance = Self {
            root_transaction: Some(conn.transaction()?),
            db_migrator_configuration,
        };
        Ok(new_instance)
    }

    fn root_transaction_ref(&self) -> &Transaction<'a> {
        self.root_transaction.as_ref().expect_v("root transaction")
    }
}

impl<'a> DBMigrationUtilities for DBMigrationUtilitiesReal<'a> {
    fn update_schema_version(&self, updated_to: String) -> rusqlite::Result<()> {
        DbMigratorReal::update_schema_version(
            self.db_migrator_configuration
                .db_configuration_table
                .as_str(),
            &self.root_transaction_ref(),
            updated_to,
        )
    }

    fn execute_upon_transaction(&self, sql_statements: &[&'static str]) -> rusqlite::Result<()> {
        let transaction = self.root_transaction_ref();
        sql_statements.iter().fold(Ok(()), |so_far, stm| {
            if let Ok(_) = so_far {
                transaction.execute(stm, NO_PARAMS).map(|_| ())
            } else {
                so_far
            }
        })
    }

    fn commit(&mut self) -> Result<(), String> {
        self.root_transaction
            .take()
            .expect_v("owned root transaction")
            .commit()
            .map_err(|e| e.to_string())
    }
}

struct DBMigratorConfiguration {
    db_configuration_table: String,
}

impl DBMigratorConfiguration {
    fn new() -> Self {
        DBMigratorConfiguration {
            db_configuration_table: "config".to_string(),
        }
    }
}

//define a new update here and add it to this list: 'list_of_existing_updates()'
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
#[allow(non_camel_case_types)]
struct Migrate_0_to_1;

impl DatabaseMigration for Migrate_0_to_1 {
    fn migrate(&self, mig_utils: &dyn DBMigrationUtilities) -> rusqlite::Result<()> {
        mig_utils.execute_upon_transaction(&[
            "INSERT INTO config (name, value, encrypted) VALUES ('mapping_protocol', null, 0)",
            //another statement would follow here
        ])
    }

    fn old_version(&self) -> usize {
        0
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

impl DbMigratorReal {
    pub fn new() -> Self {
        Self {
            logger: Logger::new("DbMigrator"),
        }
    }

    fn list_of_existing_updates<'a>() -> &'a [&'a dyn DatabaseMigration] {
        &[&Migrate_0_to_1]
    }

    fn make_updates<'a>(
        &self,
        mismatched_schema: usize,
        target_version: usize,
        mut migration_utilities: Box<dyn DBMigrationUtilities + 'a>,
        list_of_updates: &'a [&'a (dyn DatabaseMigration + 'a)],
    ) -> Result<(), String> {
        let updates_to_process =
            Self::select_updates_to_process(mismatched_schema, list_of_updates);
        let mut peekable_list = updates_to_process.iter().peekable();
        for _ in 0..peekable_list.len() {
            let (next_record, updatable_to) = Self::process_items_with_dirty_references(
                peekable_list.next(),
                peekable_list.peek(),
            );
            let current_state = next_record.old_version();
            //necessary for testing
            if Self::is_target_version_reached(current_state, &target_version) {
                return Ok(());
            }
            let versions_in_question =
                Self::context_between_two_versions(current_state, &updatable_to);

            if let Err(e) =
                Self::migrate_semi_automated(next_record, updatable_to, &*migration_utilities)
            {
                return self.dispatch_bad_news(&versions_in_question, e);
            }
            self.log_success(&versions_in_question)
        }
        migration_utilities.commit()
    }

    fn migrate_semi_automated<'a>(
        record: &dyn DatabaseMigration,
        updated_to: String,
        migration_utilities: &dyn DBMigrationUtilities,
    ) -> rusqlite::Result<()> {
        record.migrate(migration_utilities)?;
        migration_utilities.update_schema_version(updated_to)
    }

    fn update_schema_version(
        name_of_given_table: &str,
        transaction: &Transaction,
        updated_to: String,
    ) -> rusqlite::Result<()> {
        transaction.execute(
            &format!(
                "UPDATE {} SET value = ? WHERE name = 'schema_version'",
                name_of_given_table
            ),
            &[updated_to],
        )?;
        Ok(())
    }

    fn is_target_version_reached(current_state: usize, target_version: &usize) -> bool {
        if current_state.lt(target_version) {
            return false;
        } else if current_state.eq(target_version) {
            return true;
        } else {
            panic!("Nonsense: the given target is lower than the version that is considered mismatched")
        }
    }

    fn select_updates_to_process<'a>(
        mismatched_schema: usize,
        list_of_updates: &'a [&'a (dyn DatabaseMigration + 'a)],
    ) -> Vec<&'a (dyn DatabaseMigration + 'a)> {
        let updates_to_process = list_of_updates
            .iter()
            .skip_while(|entry| entry.old_version().ne(&mismatched_schema))
            .map(Self::deref)
            .collect::<Vec<&(dyn DatabaseMigration + 'a)>>();
        let _ = Self::check_out_quantity_of_those_remaining(
            mismatched_schema,
            updates_to_process.len(),
        );
        updates_to_process
    }

    fn deref<'a>(value: &'a &dyn DatabaseMigration) -> &'a dyn DatabaseMigration {
        *value
    }

    fn process_items_with_dirty_references<'a>(
        first: Option<&'a &dyn DatabaseMigration>,
        second: Option<&&&dyn DatabaseMigration>,
    ) -> (&'a dyn DatabaseMigration, String) {
        let first = *first.expect_v("migration record");
        let identity_of_the_second = Self::identify_the_next_record(second, first);
        (first, identity_of_the_second)
    }

    fn identify_the_next_record(
        subject: Option<&&&dyn DatabaseMigration>,
        current_record: &dyn DatabaseMigration,
    ) -> String {
        if let Some(next_higher) = subject {
            next_higher.old_version().to_string()
        } else {
            (current_record.old_version() + 1).to_string()
        }
    }

    fn check_out_quantity_of_those_remaining(mismatched_schema: usize, count: usize) {
        if count == 0 {
            panic!("Database claims to be more advanced ({}) than the version {} which is the latest released.", mismatched_schema, CURRENT_SCHEMA_VERSION)
        }
    }

    fn dispatch_bad_news(&self, versions: &str, error: rusqlite::Error) -> Result<(), String> {
        let error_message = format!("Updating database {} failed: {:?}", versions, error);
        warning!(self.logger, "{}", &error_message);
        Err(error_message)
    }

    fn context_between_two_versions(first: usize, second: &str) -> String {
        format!("from version {} to {}", first, second)
    }

    fn log_success(&self, versions: &str) {
        info!(self.logger, "Database successfully updated {}", versions)
    }

    fn compare_two_numbers(first: usize, second: usize) -> bool {
        first.le(&second)
    }
}

#[cfg(test)]
mod tests {
    use crate::database::connection_wrapper::{ConnectionWrapper, ConnectionWrapperReal};
    use crate::database::db_initializer::test_utils::ConnectionWrapperMock;
    use crate::database::db_initializer::{
        DbInitializer, DbInitializerReal, CURRENT_SCHEMA_VERSION, DATABASE_FILE,
    };
    use crate::database::db_migrations::{
        DBMigrationUtilities, DBMigrationUtilitiesReal, DatabaseMigration, DbMigrator,
        Migrate_0_to_1,
    };
    use crate::database::db_migrations::{DBMigratorConfiguration, DbMigratorReal};
    use crate::test_utils::database_utils::{
        assurance_query_for_config_table,
        revive_tables_of_the_version_0_and_return_connection_to_the_db,
    };
    use crate::test_utils::logging::{init_test_logging, TestLogHandler};
    use lazy_static::lazy_static;
    use masq_lib::test_utils::utils::{BASE_TEST_DIR, DEFAULT_CHAIN_ID};
    use rusqlite::{Connection, Error, NO_PARAMS};
    use std::cell::RefCell;
    use std::fmt::Debug;
    use std::fs::create_dir_all;
    use std::ops::Not;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct DBMigrationUtilitiesMock {
        update_schema_version_params: Arc<Mutex<Vec<String>>>,
        update_schema_version_results: RefCell<Vec<rusqlite::Result<()>>>,
        execute_upon_transaction_params: Arc<Mutex<Vec<Vec<String>>>>,
        execute_upon_transaction_result: RefCell<Vec<rusqlite::Result<()>>>,
        commit_params: Arc<Mutex<Vec<()>>>,
        commit_results: RefCell<Vec<Result<(), String>>>,
    }

    impl DBMigrationUtilitiesMock {
        pub fn update_schema_version_params(mut self, params: &Arc<Mutex<Vec<String>>>) -> Self {
            self.update_schema_version_params = params.clone();
            self
        }

        pub fn update_schema_version_result(self, result: rusqlite::Result<()>) -> Self {
            self.update_schema_version_results.borrow_mut().push(result);
            self
        }

        pub fn execute_upon_transaction_params(
            mut self,
            params: &Arc<Mutex<Vec<Vec<String>>>>,
        ) -> Self {
            self.execute_upon_transaction_params = params.clone();
            self
        }

        pub fn execute_upon_transaction_result(self, result: rusqlite::Result<()>) -> Self {
            self.execute_upon_transaction_result
                .borrow_mut()
                .push(result);
            self
        }

        pub fn commit_params(mut self, params: &Arc<Mutex<Vec<()>>>) -> Self {
            self.commit_params = params.clone();
            self
        }

        pub fn commit_result(self, result: Result<(), String>) -> Self {
            self.commit_results.borrow_mut().push(result);
            self
        }
    }

    impl DBMigrationUtilities for DBMigrationUtilitiesMock {
        fn update_schema_version(&self, updated_to: String) -> rusqlite::Result<()> {
            self.update_schema_version_params
                .lock()
                .unwrap()
                .push(updated_to);
            self.update_schema_version_results.borrow_mut().remove(0)
        }

        fn execute_upon_transaction(
            &self,
            sql_statements: &[&'static str],
        ) -> rusqlite::Result<()> {
            self.execute_upon_transaction_params.lock().unwrap().push(
                sql_statements
                    .iter()
                    .map(|str| str.to_string())
                    .collect::<Vec<String>>(),
            );
            self.execute_upon_transaction_result.borrow_mut().remove(0)
        }

        fn commit(&mut self) -> Result<(), String> {
            self.commit_params.lock().unwrap().push(());
            self.commit_results.borrow_mut().remove(0)
        }
    }

    lazy_static! {
        static ref TEST_DIRECTORY_FOR_DB_MIGRATION: PathBuf =
            PathBuf::from(format!("{}/db_migration", BASE_TEST_DIR));

        static ref CHANGABLE_PANIC_MESSAGE:String = format!("Database claims to be more advanced ({}) than the version {} which is the latest released.",CURRENT_SCHEMA_VERSION + 1,CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn migrate_database_handles_an_error_from_creating_the_root_transaction() {
        let subject = DbMigratorReal::new();
        let mismatched_schema = 0;
        let target_version = 5; //not relevant
        let connection = ConnectionWrapperMock::default()
            .transaction_result(Err(Error::SqliteSingleThreadedMode)); //hard to find a real-like error for this

        let result =
            subject.migrate_database(mismatched_schema, target_version, Box::new(connection));

        assert_eq!(
            result,
            Err("SQLite was compiled or configured for single-threaded use only".to_string())
        )
    }

    #[test]
    fn make_updates_panics_if_the_given_schema_is_of_higher_number_than_the_latest_official() {
        let last_version = CURRENT_SCHEMA_VERSION;
        let too_advanced = last_version + 1;
        let migration_utilities = DBMigrationUtilitiesMock::default();
        let target_version = 5; //not relevant
        let subject = DbMigratorReal::default();

        let captured_panic = catch_unwind(AssertUnwindSafe(|| {
            subject.make_updates(
                too_advanced,
                target_version,
                Box::new(migration_utilities),
                DbMigratorReal::list_of_existing_updates(),
            )
        }))
        .unwrap_err();

        let panic_message = captured_panic.downcast_ref::<String>().unwrap();
        assert_eq!(*panic_message,format!("Database claims to be more advanced ({}) than the version {} which is the latest released.",too_advanced,CURRENT_SCHEMA_VERSION))
    }

    #[derive(Default, Debug)]
    struct DBMigrationRecordMock {
        old_version_params: Arc<Mutex<Vec<()>>>,
        old_version_result: RefCell<Vec<usize>>,
        migrate_params: Arc<Mutex<Vec<()>>>,
        migrate_result: RefCell<Vec<rusqlite::Result<()>>>,
    }

    impl DBMigrationRecordMock {
        fn old_version_params(mut self, params: &Arc<Mutex<Vec<()>>>) -> Self {
            self.old_version_params = params.clone();
            self
        }
        fn old_version_result(self, result: usize) -> Self {
            self.old_version_result.borrow_mut().push(result);
            self
        }

        fn migrate_result(self, result: rusqlite::Result<()>) -> Self {
            self.migrate_result.borrow_mut().push(result);
            self
        }

        fn migrate_params(mut self, params: &Arc<Mutex<Vec<()>>>) -> Self {
            self.migrate_params = params.clone();
            self
        }

        fn set_full_tooling_for_mock_migration_record(
            self,
            result_o_v: usize,
            params_o_v: &Arc<Mutex<Vec<()>>>,
            result_m: rusqlite::Result<()>,
            params_m: &Arc<Mutex<Vec<()>>>,
        ) -> Self {
            self.old_version_result(result_o_v)
                .old_version_params(params_o_v)
                .migrate_result(result_m)
                .migrate_params(params_m)
        }
    }

    impl DatabaseMigration for DBMigrationRecordMock {
        fn migrate(&self, _migration_utilities: &dyn DBMigrationUtilities) -> rusqlite::Result<()> {
            self.migrate_params.lock().unwrap().push(());
            self.migrate_result.borrow_mut().remove(0)
        }

        fn old_version(&self) -> usize {
            self.old_version_params.lock().unwrap().push(());
            self.old_version_result.borrow()[0]
        }
    }

    #[test]
    #[should_panic(expected = "The list of updates for the database is not ordered properly")]
    fn list_validation_check_works() {
        let fake_one = DBMigrationRecordMock::default().old_version_result(6);
        let fake_two = DBMigrationRecordMock::default().old_version_result(3);
        let list: &[&dyn DatabaseMigration] = &[&Migrate_0_to_1, &fake_one, &fake_two];

        let _ = list_validation_check(list);
    }

    fn list_validation_check<'a>(list_of_updates: &'a [&'a (dyn DatabaseMigration + 'a)]) {
        let iterator = list_of_updates.iter();
        let iterator_shifted = list_of_updates.iter().skip(1);
        iterator.zip(iterator_shifted).for_each(|(first, second)| {
            if DbMigratorReal::compare_two_numbers(first.old_version(), second.old_version()).not()
            {
                panic!("The list of updates for the database is not ordered properly")
            }
        });
    }

    #[test]
    fn list_of_existing_updates_is_correctly_ordered() {
        let _ = list_validation_check(DbMigratorReal::list_of_existing_updates());
        //success if no panicking
    }

    #[test]
    fn list_of_existing_updates_does_not_end_with_version_higher_than_the_current_version() {
        let last_entry = DbMigratorReal::list_of_existing_updates()
            .into_iter()
            .last();

        let result = last_entry.unwrap().old_version();

        assert!(DbMigratorReal::compare_two_numbers(
            result,
            CURRENT_SCHEMA_VERSION
        ))
    }

    #[test]
    fn migrate_semi_automated_returns_an_error_from_update_schema_version() {
        let update_schema_version_params_arc = Arc::new(Mutex::new(vec![]));
        let mut migration_record = DBMigrationRecordMock::default().migrate_result(Ok(()));
        let migration_utilities = DBMigrationUtilitiesMock::default()
            .update_schema_version_result(Err(Error::InvalidQuery))
            .update_schema_version_params(&update_schema_version_params_arc);

        let result = DbMigratorReal::migrate_semi_automated(
            &mut migration_record,
            5.to_string(),
            &migration_utilities,
        );

        assert_eq!(result, Err(Error::InvalidQuery));
        let update_schema_version_params = update_schema_version_params_arc.lock().unwrap();
        assert_eq!(*update_schema_version_params, vec!["5".to_string()])
    }

    #[test]
    fn migrate_semi_automated_returns_an_error_from_migrate() {
        let mut migration_record =
            DBMigrationRecordMock::default().migrate_result(Err(Error::InvalidColumnIndex(5)));
        let migration_utilities = DBMigrationUtilitiesMock::default();

        let result = DbMigratorReal::migrate_semi_automated(
            &mut migration_record,
            5.to_string(),
            &migration_utilities,
        );

        assert_eq!(result, Err(Error::InvalidColumnIndex(5)));
    }

    #[test]
    fn execute_upon_transaction_returns_the_first_error_encountered_and_the_transaction_is_canceled(
    ) {
        let dir_path: PathBuf = TEST_DIRECTORY_FOR_DB_MIGRATION
            .join("execute_upon_transaction_returns_the_first_error_encountered_and_the_transaction_is_canceled");
        create_dir_all(&dir_path).unwrap();
        let db_path = dir_path.join("test_database.db");
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS test (
            name TEXT,
            count TEXT
        )",
                NO_PARAMS,
            )
            .unwrap();
        let correct_statement_1 = "INSERT INTO test (name,count) VALUES ('mushrooms','270')";
        let erroneous_statement_1 = "INSERT INTO botanic_garden (sun_flowers) VALUES (100)";
        let erroneous_statement_2 = "UPDATE botanic_garden SET (sun_flowers) VALUES (99)";
        let set_of_sql_statements = &[
            correct_statement_1,
            erroneous_statement_1,
            erroneous_statement_2,
        ];
        let mut connection_wrapper = ConnectionWrapperReal::new(connection);
        let config = DBMigratorConfiguration::new();
        let subject = DBMigrationUtilitiesReal::new(&mut connection_wrapper, config).unwrap();

        let result = subject.execute_upon_transaction(set_of_sql_statements);

        assert_eq!(
            result.unwrap_err().to_string(),
            "no such table: botanic_garden"
        );
        let connection = Connection::open(&db_path).unwrap();
        //when an error occurs the underlying transaction gets rolled back, and we cannot see any changes to the database
        let assertion: Result<(String, String), Error> = connection.query_row(
            "SELECT count FROM test WHERE name='mushrooms'",
            NO_PARAMS,
            |row| Ok((row.get(0).unwrap(), row.get(1).unwrap())),
        );
        assert_eq!(assertion.unwrap_err().to_string(), "Query returned no rows")
    }

    #[test]
    #[should_panic(
        expected = "Nonsense: the given target is lower than the version that is considered mismatched"
    )]
    fn make_updates_panics_if_the_parameters_for_a_test_does_not_make_sense() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute(
                "CREATE TABLE test (
            name TEXT,
            value TEXT
        )",
                NO_PARAMS,
            )
            .unwrap();
        let mut connection_wrapper = ConnectionWrapperReal::new(connection);
        let config = DBMigratorConfiguration::new();
        let subject = DbMigratorReal::new();
        let list_of_updates: &[&dyn DatabaseMigration] =
            &[&DBMigrationRecordMock::default().old_version_result(5)];

        let _ = subject.make_updates(
            5,
            3,
            Box::new(DBMigrationUtilitiesReal::new(&mut connection_wrapper, config).unwrap()),
            list_of_updates,
        );
    }

    #[test]
    fn final_commit_of_the_root_transaction_sad_path() {
        let first_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let second_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let first_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let second_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let commit_params_arc = Arc::new(Mutex::new(vec![]));
        let list_of_updates: &[&dyn DatabaseMigration] = &[
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                0,
                &first_record_old_version_p_arc,
                Ok(()),
                &first_record_migration_p_arc,
            ),
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                1,
                &second_record_old_version_p_arc,
                Ok(()),
                &second_record_migration_p_arc,
            ),
        ];
        let migration_utils = DBMigrationUtilitiesMock::default()
            .update_schema_version_result(Ok(()))
            .update_schema_version_result(Ok(()))
            .commit_params(&commit_params_arc)
            .commit_result(Err("Committing transaction failed".to_string()));
        let subject = DbMigratorReal::new();

        let result = subject.make_updates(0, 2, Box::new(migration_utils), list_of_updates);

        assert_eq!(result, Err(String::from("Committing transaction failed")));
        let first_record_old_version_param = first_record_old_version_p_arc.lock().unwrap();
        assert_eq!(first_record_old_version_param.len(), 2);
        let second_record_old_version_param = second_record_old_version_p_arc.lock().unwrap();
        assert_eq!(second_record_old_version_param.len(), 3);
        let first_record_migration_params = first_record_migration_p_arc.lock().unwrap();
        assert_eq!(*first_record_migration_params, vec![()]);
        let second_record_migration_params = second_record_migration_p_arc.lock().unwrap();
        assert_eq!(*second_record_migration_params, vec![()]);
        let commit_params = commit_params_arc.lock().unwrap();
        assert_eq!(*commit_params, vec![()])
    }

    #[test]
    fn make_updates_skips_records_already_included_in_the_current_database_and_updates_only_the_others(
    ) {
        let first_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let second_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let third_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let fourth_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let fifth_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let first_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let second_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let third_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let fourth_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let fifth_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let list_of_updates: &[&dyn DatabaseMigration] = &[
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                0,
                &first_record_old_version_p_arc,
                Ok(()),
                &first_record_migration_p_arc,
            ),
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                1,
                &second_record_old_version_p_arc,
                Ok(()),
                &second_record_migration_p_arc,
            ),
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                2,
                &third_record_old_version_p_arc,
                Ok(()),
                &third_record_migration_p_arc,
            ),
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                3,
                &fourth_record_old_version_p_arc,
                Ok(()),
                &fourth_record_migration_p_arc,
            ),
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                4,
                &fifth_record_old_version_p_arc,
                Ok(()),
                &fifth_record_migration_p_arc,
            ),
        ];
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute(
                "CREATE TABLE test (
            name TEXT,
            value TEXT
        )",
                NO_PARAMS,
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO test (name, value) VALUES ('schema_version', '3')",
                NO_PARAMS,
            )
            .unwrap();
        let mut connection_wrapper = ConnectionWrapperReal::new(connection);
        let config = DBMigratorConfiguration {
            db_configuration_table: "test".to_string(),
        };
        let subject = DbMigratorReal::new();

        let result = subject.make_updates(
            2,
            5,
            Box::new(DBMigrationUtilitiesReal::new(&mut connection_wrapper, config).unwrap()),
            list_of_updates,
        );

        assert_eq!(result, Ok(()));
        let first_record_old_version_param = first_record_old_version_p_arc.lock().unwrap();
        assert_eq!(first_record_old_version_param.len(), 1);
        let second_record_old_version_param = second_record_old_version_p_arc.lock().unwrap();
        assert_eq!(second_record_old_version_param.len(), 1);
        let third_record_old_version_param = third_record_old_version_p_arc.lock().unwrap();
        assert_eq!(third_record_old_version_param.len(), 2);
        let fourth_record_old_version_param = fourth_record_old_version_p_arc.lock().unwrap();
        assert_eq!(fourth_record_old_version_param.len(), 2);
        let fifth_record_old_version_param = fifth_record_old_version_p_arc.lock().unwrap();
        assert_eq!(fifth_record_old_version_param.len(), 3);
        let first_record_migration_params = first_record_migration_p_arc.lock().unwrap();
        assert_eq!(*first_record_migration_params, vec![]);
        let second_record_migration_params = second_record_migration_p_arc.lock().unwrap();
        assert_eq!(*second_record_migration_params, vec![]);
        let third_record_migration_params = third_record_migration_p_arc.lock().unwrap();
        assert_eq!(*third_record_migration_params, vec![()]);
        let fourth_record_migration_params = fourth_record_migration_p_arc.lock().unwrap();
        assert_eq!(*fourth_record_migration_params, vec![()]);
        let fifth_record_migration_params = fifth_record_migration_p_arc.lock().unwrap();
        assert_eq!(*fifth_record_migration_params, vec![()]);
        let assertion: (String, String) = connection_wrapper
            .transaction()
            .unwrap()
            .query_row(
                "SELECT name, value FROM test WHERE name='schema_version'",
                NO_PARAMS,
                |row| Ok((row.get(0).unwrap(), row.get(1).unwrap())),
            )
            .unwrap();
        assert_eq!(assertion.1, "5")
    }

    #[test]
    fn make_updates_stops_doing_updates_on_the_version_specified_as_a_parameter() {
        let first_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let second_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let third_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let fourth_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let fifth_record_old_version_p_arc = Arc::new(Mutex::new(vec![]));
        let first_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let second_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let third_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let fourth_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let fifth_record_migration_p_arc = Arc::new(Mutex::new(vec![]));
        let list_of_updates: &[&dyn DatabaseMigration] = &[
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                0,
                &first_record_old_version_p_arc,
                Ok(()),
                &first_record_migration_p_arc,
            ),
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                1,
                &second_record_old_version_p_arc,
                Ok(()),
                &second_record_migration_p_arc,
            ),
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                2,
                &third_record_old_version_p_arc,
                Ok(()),
                &third_record_migration_p_arc,
            ),
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                3,
                &fourth_record_old_version_p_arc,
                Ok(()),
                &fourth_record_migration_p_arc,
            ),
            &DBMigrationRecordMock::default().set_full_tooling_for_mock_migration_record(
                4,
                &fifth_record_old_version_p_arc,
                Ok(()),
                &fifth_record_migration_p_arc,
            ),
        ];
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute(
                "CREATE TABLE test (
            name TEXT,
            value TEXT
        )",
                NO_PARAMS,
            )
            .unwrap();
        let mut connection_wrapper = ConnectionWrapperReal::new(connection);
        let config = DBMigratorConfiguration {
            db_configuration_table: "test".to_string(),
        };
        let subject = DbMigratorReal::new();

        let result = subject.make_updates(
            0,
            2,
            Box::new(DBMigrationUtilitiesReal::new(&mut connection_wrapper, config).unwrap()),
            list_of_updates,
        );

        assert_eq!(result, Ok(()));
        let first_record_old_version_param = first_record_old_version_p_arc.lock().unwrap();
        assert_eq!(first_record_old_version_param.len(), 2);
        let second_record_old_version_param = second_record_old_version_p_arc.lock().unwrap();
        assert_eq!(second_record_old_version_param.len(), 2);
        let third_record_old_version_param = third_record_old_version_p_arc.lock().unwrap();
        assert_eq!(third_record_old_version_param.len(), 2);
        let fourth_record_old_version_param = fourth_record_old_version_p_arc.lock().unwrap();
        assert_eq!(fourth_record_old_version_param.len(), 1);
        let fifth_record_old_version_param = fifth_record_old_version_p_arc.lock().unwrap();
        assert_eq!(fifth_record_old_version_param.len(), 0);
        let first_record_migration_params = first_record_migration_p_arc.lock().unwrap();
        assert_eq!(*first_record_migration_params, vec![()]);
        let second_record_migration_params = second_record_migration_p_arc.lock().unwrap();
        assert_eq!(*second_record_migration_params, vec![()]);
        let third_record_migration_params = third_record_migration_p_arc.lock().unwrap();
        assert_eq!(*third_record_migration_params, vec![]);
        let fourth_record_migration_params = fourth_record_migration_p_arc.lock().unwrap();
        assert_eq!(*fourth_record_migration_params, vec![]);
        let fifth_record_migration_params = fifth_record_migration_p_arc.lock().unwrap();
        assert_eq!(*fifth_record_migration_params, vec![]);
    }

    #[test]
    fn db_migration_happy_path() {
        init_test_logging();
        let execute_upon_transaction_params_arc = Arc::new(Mutex::new(vec![]));
        let update_schema_version_params_arc = Arc::new(Mutex::new(vec![]));
        let commit_params_arc = Arc::new(Mutex::new(vec![]));
        let outdated_schema = 0;
        let list = &[&Migrate_0_to_1 as &dyn DatabaseMigration];
        let migration_utils = DBMigrationUtilitiesMock::default()
            .execute_upon_transaction_params(&execute_upon_transaction_params_arc)
            .execute_upon_transaction_result(Ok(()))
            .update_schema_version_params(&update_schema_version_params_arc)
            .update_schema_version_result(Ok(()))
            .commit_params(&commit_params_arc)
            .commit_result(Ok(()));
        let target_version = 5; //not relevant
        let subject = DbMigratorReal::default();

        let result = subject.make_updates(
            outdated_schema,
            target_version,
            Box::new(migration_utils),
            list,
        );

        assert!(result.is_ok());
        let execute_upon_transaction_params = execute_upon_transaction_params_arc.lock().unwrap();
        assert_eq!(
            *execute_upon_transaction_params[0],
            vec![
                "INSERT INTO config (name, value, encrypted) VALUES ('mapping_protocol', null, 0)"
            ]
        );
        let update_schema_version_params = update_schema_version_params_arc.lock().unwrap();
        assert_eq!(update_schema_version_params[0], "1");
        let commit_params = commit_params_arc.lock().unwrap();
        assert_eq!(commit_params[0], ());
        TestLogHandler::new().exists_log_containing(
            "INFO: DbMigrator: Database successfully updated from version 0 to 1",
        );
    }

    #[test]
    fn migration_from_0_to_1_is_properly_set() {
        let dir_path = TEST_DIRECTORY_FOR_DB_MIGRATION.join("0_to_1");
        create_dir_all(&dir_path).unwrap();
        let db_path = dir_path.join(DATABASE_FILE);
        let connection = revive_tables_of_the_version_0_and_return_connection_to_the_db(&db_path);
        let subject = DbInitializerReal::default();

        let result = subject.initialize_to_version(&dir_path, DEFAULT_CHAIN_ID, 1, true);

        let (mp_name, mp_value, mp_encrypted): (String, Option<String>, u16) =
            assurance_query_for_config_table(
                &connection,
                "select name, value, encrypted from config where name = 'mapping_protocol'",
            );
        let (cs_name, cs_value, cs_encrypted): (String, Option<String>, u16) =
            assurance_query_for_config_table(
                &connection,
                "select name, value, encrypted from config where name = 'schema_version'",
            );
        assert!(result
            .unwrap()
            .as_any()
            .downcast_ref::<ConnectionWrapperReal>()
            .is_some());
        assert_eq!(mp_name, "mapping_protocol".to_string());
        assert_eq!(mp_value, None);
        assert_eq!(mp_encrypted, 0);
        assert_eq!(cs_name, "schema_version".to_string());
        assert_eq!(cs_value, Some("1".to_string()));
        assert_eq!(cs_encrypted, 0)
    }
}
