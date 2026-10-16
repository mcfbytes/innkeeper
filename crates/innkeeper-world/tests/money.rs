//! CasinoLand bankroll transcripts (command 40 subs 2 and 3); layouts are in golden/user_info.txt.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use common::Table;
use innkeeper_world::{AccountId, Money};

/// The account the `{login}` of tests/common/mod.rs presents.
const ACCOUNT: AccountId = AccountId(100_001);
const SID: &str = "05 01";
const READ: &str = "28 03 05 01 01 00 01 00";

#[test]
fn a_first_read_reports_the_starting_bankroll_and_a_save_is_kept() {
    let mut table = Table::new();
    table.run(&format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > read the bankroll of an account the host has never seen | {READ}
        A < the starting 500 | {READ} f4 01 00 00
        A > save 247 | 28 02 {SID} 01 00 01 00 f7 00 00 00
        A < the save is acknowledged | 00 00 {SID} 28 02
        A > read it back | {READ}
        A < 247 | {READ} f7 00 00 00
        "
    ));
    assert_eq!(table.world.store.money(ACCOUNT).unwrap(), Some(Money(247)));
}

#[test]
fn a_save_above_or_below_the_start_is_what_the_next_read_returns() {
    let mut table = Table::new();
    table.run(&format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > save 499 | 28 02 {SID} 01 00 01 00 f3 01 00 00
        A < the save is acknowledged | 00 00 {SID} 28 02
        A > read it back | {READ}
        A < 499 | {READ} f3 01 00 00
        A > save 750 | 28 02 {SID} 01 00 01 00 ee 02 00 00
        A < the save is acknowledged | 00 00 {SID} 28 02
        A > read the higher balance | {READ}
        A < 750 | {READ} ee 02 00 00
        "
    ));
    assert_eq!(table.world.store.money(ACCOUNT).unwrap(), Some(Money(750)));
}

#[test]
fn a_stored_balance_is_not_replaced_by_the_starting_amount() {
    let mut table = Table::new();
    table
        .world
        .store
        .set_money(ACCOUNT, Money(1_000_000))
        .unwrap();
    table.run(&format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > read the bankroll | {READ}
        A < the stored million | {READ} 40 42 0f 00
        "
    ));
}

#[test]
fn a_saved_bankroll_outlives_the_connection() {
    let mut table = Table::new();
    table.run(&format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > save 247 | 28 02 {SID} 01 00 01 00 f7 00 00 00
        A < the save is acknowledged | 00 00 {SID} 28 02
        A ! hang up
        A > login again | {{login}}
        A < login ack | {{login ack}}
        A > read the bankroll | {READ}
        A < still 247 | {READ} f7 00 00 00
        "
    ));
}

#[test]
fn a_bankroll_of_zero_is_not_the_same_as_none() {
    let mut table = Table::new();
    table.run(&format!(
        "
        A > login | {{login}}
        A < login ack | {{login ack}}
        A > save nothing | 28 02 {SID} 01 00 01 00 00 00 00 00
        A < the save is acknowledged | 00 00 {SID} 28 02
        A > read the bankroll | {READ}
        A < 0, not the starting amount | {READ} 00 00 00 00
        "
    ));
}

#[test]
fn a_request_before_login_touches_nothing() {
    let mut table = Table::new();
    table.run(&format!(
        "
        A > save before login | 28 02 {SID} 01 00 01 00 f7 00 00 00
        "
    ));
    assert_eq!(table.world.store.money(ACCOUNT).unwrap(), None);
}
