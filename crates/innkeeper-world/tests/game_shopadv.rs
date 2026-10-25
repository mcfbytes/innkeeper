//! Shoppers Advantage from the INN map, closed edition: the whole sequence of one stock 2.3 client
//! and SHOPADV.EXE, in the order of the live run of 2026-10-25 (games.md section 11.12). The client
//! bytes are from the host's log of that run; the host bytes are this host's replies (the closed
//! text is an assumption, see `assumptions.rs`). Line syntax is in common/mod.rs.
//!
//! `Table` refuses a client message while an earlier reply is still unchecked, so the program
//! switch is covered too: nothing is sent between the `54/19` that ends LSCI and SHOPADV's JOIN
//! (`shopadv.md` section 11 item 8).
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

mod common;

use common::Table;

/// The gate: LSCI's `gotoCUCmall` sends `54/27` with route 0 and the host opens the mall (`54/19`
/// kind 0). The land then exits to SHOPADV.EXE; the client's next message is the JOIN below.
const GATE: &str = "
    A > gate 54/27 | 36 1b 05 00 ff ff ee ee dd dd 00 00
    A < open, 54/19 kind 0 | 36 13 05 00 ff ff ee ee dd dd 00 00 00
";

/// SHOPADV's JOIN (`54/25`, route 0xCCCC) is answered with the token (`54/17`) and the closed text
/// as a kind 1 `54/16`.
const JOIN: &str = "
    A > join 54/25 | 36 19 05 00 ff ff ee ee dd dd cc cc
    A < token 54/17 | 36 11 05 00 ff ff ee ee dd dd cc cc
    A < closed text 54/16 kind 1 | 36 10 05 00 ff ff ee ee dd dd cc cc 01 54 68 65 20 53 68 6f 70 \
70 65 72 73 20 41 64 76 61 6e 74 61 67 65 20 6d 61 6c 6c 20 69 73 20 63 6c 6f 73 65 64 2e 0a 00
";

/// A typed line, `hello\r\n`, is answered at once with the closed text (the 35 s limit).
const TYPED_LINE: &str = "
    A > text 54/24 | 36 18 05 00 ff ff ee ee dd dd cc cc 68 65 6c 6c 6f 0d 0a 00
    A < closed text 54/16 kind 1 | 36 10 05 00 ff ff ee ee dd dd cc cc 01 54 68 65 20 53 68 6f 70 \
70 65 72 73 20 41 64 76 61 6e 74 61 67 65 20 6d 61 6c 6c 20 69 73 20 63 6c 6f 73 65 64 2e 0a 00
";

/// The Exit button: TEXT `/EXIT\n` (the notice only), then EXIT `54/26`, answered with `54/19` kind
/// 0x1A.
const BUTTON_EXIT: &str = "
    A > text /EXIT, button form | 36 18 05 00 ff ff ee ee dd dd cc cc 2f 45 58 49 54 0a 00
    A < closed text 54/16 kind 1 | 36 10 05 00 ff ff ee ee dd dd cc cc 01 54 68 65 20 53 68 6f 70 \
70 65 72 73 20 41 64 76 61 6e 74 61 67 65 20 6d 61 6c 6c 20 69 73 20 63 6c 6f 73 65 64 2e 0a 00
    A > exit 54/26 | 36 1a 05 00 ff ff ee ee dd dd cc cc
    A < exit acknowledged, 54/19 kind 0x1a | 36 13 05 00 ff ff ee ee dd dd cc cc 1a
";

/// A typed `/EXIT\r\n` is not recognised by the client; the host ends the session itself with
/// `54/20` kind 0x1A after the notice. The client then sends EXIT, which is answered as above.
const TYPED_EXIT: &str = "
    A > text /EXIT, typed form | 36 18 05 00 ff ff ee ee dd dd cc cc 2f 45 58 49 54 0d 0a 00
    A < closed text 54/16 kind 1 | 36 10 05 00 ff ff ee ee dd dd cc cc 01 54 68 65 20 53 68 6f 70 \
70 65 72 73 20 41 64 76 61 6e 74 61 67 65 20 6d 61 6c 6c 20 69 73 20 63 6c 6f 73 65 64 2e 0a 00
    A < session ended, 54/20 kind 0x1a | 36 14 05 00 ff ff ee ee dd dd cc cc 1a 00 00 54 68 65 20 \
53 68 6f 70 70 65 72 73 20 41 64 76 61 6e 74 61 67 65 20 6d 61 6c 6c 20 69 73 20 63 6c 6f 73 65 \
64 2e 0a 00
    A > exit 54/26 | 36 1a 05 00 ff ff ee ee dd dd cc cc
    A < exit acknowledged, 54/19 kind 0x1a | 36 13 05 00 ff ff ee ee dd dd cc cc 1a
";

const LOGON: &str = "
    A > login | {login}
    A < login ack | {login ack}
";

#[test]
fn the_whole_visit_gate_join_text_and_the_exit_button() {
    let transcript = [LOGON, GATE, JOIN, TYPED_LINE, BUTTON_EXIT].concat();
    Table::new().run(&transcript);
}

#[test]
fn a_second_visit_leaves_with_a_typed_exit() {
    // The same connection goes round twice: the land is back after the first exit and the gate,
    // the program switch and the JOIN repeat.
    let transcript = [LOGON, GATE, JOIN, BUTTON_EXIT, GATE, JOIN, TYPED_EXIT].concat();
    Table::new().run(&transcript);
}

#[test]
fn the_gate_alone_sends_nothing_before_the_join() {
    // Only the 54/19 follows the gate; a JOIN right after it is the next message of the new program.
    let transcript = [LOGON, GATE].concat();
    Table::new().run(&transcript);
}
