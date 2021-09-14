// Copyright (c) 2019, MASQ (https://masq.ai) and/or its affiliates. All rights reserved.

#![cfg(test)]

use crate::database::connection_wrapper::ConnectionWrapper;
use crate::database::db_migrations::DbMigrator;
use itertools::Itertools;
use masq_lib::logger::Logger;
use rusqlite::Connection;
use std::cell::RefCell;
use std::collections::HashSet;
use std::env::current_dir;
use std::fs::{remove_file, File};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub fn bring_db_0_back_to_life_and_return_connection(db_path: &PathBuf) -> Connection {
    match remove_file(db_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => panic!("Unexpected but serious error: {}", e),
        _ => (),
    };
    let connection = Connection::open(&db_path).unwrap();
    [
        "create table config (
            name text not null,
            value text,
            encrypted integer not null )",
        "create unique index idx_config_name on config (name)",
        "insert into config (name, value, encrypted) values ('example_encrypted', null, 1)",
        "insert into config (name, value, encrypted) values ('clandestine_port', '2897', 0)",
        "insert into config (name, value, encrypted) values ('consuming_wallet_derivation_path', null, 0)",
        "insert into config (name, value, encrypted) values ('consuming_wallet_public_key', null, 0)",
        "insert into config (name, value, encrypted) values ('earning_wallet_address', null, 0)",
        "insert into config (name, value, encrypted) values ('schema_version', '0', 0)",
        "insert into config (name, value, encrypted) values ('seed', null, 0)",
        "insert into config (name, value, encrypted) values ('start_block', 8688171, 0)",
        "insert into config (name, value, encrypted) values ('gas_price', '1', 0)",
        "insert into config (name, value, encrypted) values ('past_neighbors', null, 1)",
        "create table payable (
                wallet_address text primary key,
                balance integer not null,
                last_paid_timestamp integer not null,
                pending_payment_transaction text null
            )",
        "create unique index idx_payable_wallet_address on payable (wallet_address)",
        "create table receivable (
                wallet_address text primary key,
                balance integer not null,
                last_received_timestamp integer not null
            )",
        "create unique index idx_receivable_wallet_address on receivable (wallet_address)",
        "create table banned ( wallet_address text primary key )",
        "create unique index idx_banned_wallet_address on banned (wallet_address)"
    ].iter().for_each(|statement|{connection.execute(statement,NO_PARAMS).unwrap();});
    connection
}

#[derive(Default)]
pub struct DbMigratorMock {
    logger: Option<Logger>,
    migrate_database_result: RefCell<Vec<Result<(), String>>>,
    migrate_database_params: Arc<Mutex<Vec<(usize, usize, Box<dyn ConnectionWrapper>)>>>,
}

impl DbMigratorMock {
    pub fn migrate_database_result(self, result: Result<(), String>) -> Self {
        self.migrate_database_result.borrow_mut().push(result);
        self
    }
    pub fn migrate_database_params(
        mut self,
        params: &Arc<Mutex<Vec<(usize, usize, Box<dyn ConnectionWrapper>)>>>,
    ) -> Self {
        self.migrate_database_params = params.clone();
        self
    }

    pub fn inject_logger(mut self) -> Self {
        self.logger = Some(Logger::new("DbMigrator"));
        self
    }
}

impl DbMigrator for DbMigratorMock {
    fn migrate_database(
        &self,
        outdated_schema: usize,
        target_version: usize,
        conn: Box<dyn ConnectionWrapper>,
    ) -> Result<(), String> {
        self.migrate_database_params
            .lock()
            .unwrap()
            .push((outdated_schema, target_version, conn));
        self.migrate_database_result.borrow_mut().pop().unwrap()
    }
}

pub fn retrieve_config_row(conn: &dyn ConnectionWrapper, name: &str) -> (Option<String>, bool) {
    let sql = "select value, encrypted from config where name = ?";
    let mut statement = conn.prepare(sql).unwrap();
    statement
        .query_row([name], |r| {
            let value_opt: Option<String> = r.get(0).unwrap();
            let encrypted_num: u64 = r.get(1).unwrap();
            let encrypted_flag = match encrypted_num {
                0 => false,
                1 => true,
                x => panic!("Encrypted flag must be 0 or 1, not {}", x),
            };
            Ok((value_opt, encrypted_flag))
        })
        .unwrap_or_else(|e| {
            panic!(
                "panicked at {} for statement: {} on parameter '{}'",
                e, sql, name
            )
        })
}

pub fn query_specific_schema_information(
    conn: &dyn ConnectionWrapper,
    query_object: &str,
) -> Vec<String> {
    let mut table_stm = conn
        .prepare(&format!(
            "SELECT sql FROM sqlite_master WHERE type='{}'",
            query_object
        ))
        .unwrap();
    table_stm
        .query_map([], |row| Ok(row.get::<usize, Option<String>>(0).unwrap()))
        .unwrap()
        .flatten()
        .flatten()
        .collect()
}

pub fn assert_create_table_statement_contains_all_important_parts(
    conn: &dyn ConnectionWrapper,
    table_name: &str,
    expected_sql_chopped: &[&[&str]],
) {
    assert_sql_statements_contain_important_parts(
        parse_sql_to_pieces(&fetch_table_sql(conn, table_name)),
        expected_sql_chopped,
    )
}

pub fn assert_index_statement_is_coupled_with_right_parameter(
    conn: &dyn ConnectionWrapper,
    index_name: &str,
    expected_sql_chopped: &[&[&str]],
) {
    assert_sql_statements_contain_important_parts(
        parse_sql_to_pieces(&fetch_index_sql(conn, index_name)),
        expected_sql_chopped,
    )
}

pub fn assert_no_index_exists_for_table(conn: &dyn ConnectionWrapper, table_name: &str) {
    let found_indexes = query_specific_schema_information(conn, "index");
    let isolated_table_name = format!(" {} ", table_name);
    found_indexes.iter().for_each(|index_stm| {
        assert!(
            !index_stm.contains(&isolated_table_name),
            "unexpected index on this table: {}",
            index_stm
        )
    })
}

fn assert_sql_statements_contain_important_parts(
    actual: Vec<HashSet<String>>,
    expected: &[&[&str]],
) {
    let mut prepared_expected = expected.into_iter().map(|slice_of_strs| {
        HashSet::from_iter(slice_of_strs.into_iter().map(|str| str.to_string()))
    });
    actual.into_iter().for_each(|hash_set| {
        assert!(
            prepared_expected
                .find(|hash_set_expected| hash_set
                    .symmetric_difference(&hash_set_expected)
                    .collect_vec()
                    .is_empty())
                .is_some(),
            "part of the fetched statement (one line) that cannot \
                     be found in the template (key words unsorted): {:?}",
            hash_set
        )
    })
}

//prepares collections of isolated key words from a column declaration, by lines
fn parse_sql_to_pieces(sql: &str) -> Vec<HashSet<String>> {
    let body: String = sql
        .chars()
        .skip_while(|char| char != &'(')
        .skip(1)
        .take_while(|char| char != &')')
        .collect();
    let lines = body.split(',');
    lines
        .map(|line| {
            HashSet::from_iter(
                line.split(|char: char| char.is_whitespace())
                    .filter(|chunk| !chunk.is_empty())
                    .map(|chunk| chunk.to_string()),
            )
        })
        .collect()
}

fn fetch_table_sql(conn: &dyn ConnectionWrapper, specific_table: &str) -> String {
    let found_table_sqls = query_specific_schema_information(conn, "table");
    let specific_table_isolated = format!(" {} ", specific_table);
    select_desired_sql_element(found_table_sqls, &specific_table_isolated)
}

fn fetch_index_sql(conn: &dyn ConnectionWrapper, index_name: &str) -> String {
    let found_indexes = query_specific_schema_information(conn, "index");
    let index_name_isolated = format!(" {} ", index_name);
    select_desired_sql_element(found_indexes, &index_name_isolated)
}

fn select_desired_sql_element(found_elements: Vec<String>, searched_element_name: &str) -> String {
    let mut wanted_element_lowercase: Vec<String> = found_elements
        .into_iter()
        .flat_map(|element| {
            let introducing_part: String =
                element.chars().take_while(|char| char != &'(').collect();
            if introducing_part.contains(searched_element_name) {
                Some(element.to_lowercase())
            } else {
                None
            }
        })
        .collect();
    if wanted_element_lowercase.len() != 1 {
        panic!("search failed, we should've found any matching element")
    }
    wanted_element_lowercase.remove(0)
}
