//! The Town Hall services: the account texts (command 34), the service mailboxes the system list
//! names, and the two forms sent to them (37/30). The behaviour and its assumptions are described in
//! docs/server/services.md; every text is this host's own.

use tracing::{info, warn};

use crate::assumptions::{
    ACCOUNTING_BOX_ASSUMED, ACCOUNT_FORMS_ARE_DISCARDED_ASSUMED,
    FIRST_RATES_REQUEST_IS_THE_LOGON_ASSUMED, MEMBER_SERVICES_BOX_ASSUMED,
    MEMBER_SERVICES_MESSAGES_ARE_KEPT_ASSUMED, MEMBER_SERVICES_MESSAGE_LIMIT_ASSUMED,
    MEMBER_SERVICES_SECTION_ASSUMED,
};
use crate::bbs::{board_id, COMPLAINT_BOARD_LOCATION};
use crate::{
    Account, AccountReply, AccountRequest, BoardRef, HostMessage, LandType, MailFault, MailReply,
    OutgoingLetter, ServiceKind, StoreError, SummaryKind, SystemService, World,
};

const MEMBER_SERVICES_NAME: &str = "Member_Services";
const ACCOUNTING_NAME: &str = "Accounting";

const RATES_TEXT: &str =
    "Rates: none whatsoever!\nNo charges. The meter is\nfast asleep, and a volunteer\nis guarding it.";

/// The records of the 37/32 system list: the two boxes the Town Hall forms are addressed to. The
/// client keeps them in `syslst.dat`, and without them "Ask Member Services" says the service is
/// temporarily not available (`TSNMemberServices::doit`, `hub/script.050`).
pub(crate) fn system_list() -> Vec<SystemService> {
    vec![
        SystemService {
            mailbox: MEMBER_SERVICES_BOX_ASSUMED,
            kind: ServiceKind::MemberServices,
            name: MEMBER_SERVICES_NAME.to_owned(),
        },
        SystemService {
            mailbox: ACCOUNTING_BOX_ASSUMED,
            kind: ServiceKind::Accounting,
            name: ACCOUNTING_NAME.to_owned(),
        },
    ]
}

/// The reply to one command 34 request. `logon_rates_seen` is the session's memory of the first
/// 34/4, which is the logon's and stays unanswered (`FIRST_RATES_REQUEST_IS_THE_LOGON_ASSUMED`).
pub(crate) fn account_request(
    account: &Account,
    logon_rates_seen: &mut bool,
    request: &AccountRequest,
) -> Vec<HostMessage> {
    let reply = |reply| vec![HostMessage::Account(reply)];
    match *request {
        AccountRequest::Rates { sid } => {
            if FIRST_RATES_REQUEST_IS_THE_LOGON_ASSUMED && !*logon_rates_seen {
                *logon_rates_seen = true;
                info!("rates asked at logon: left unanswered");
                return Vec::new();
            }
            reply(AccountReply::Rates {
                to: sid,
                text: RATES_TEXT.to_owned(),
            })
        }
        AccountRequest::Status { sid } => reply(AccountReply::Status {
            to: sid,
            text: status_text(account),
        }),
        AccountRequest::RatesSeen { .. } => {
            info!("rates seen: absorbed");
            Vec::new()
        }
        AccountRequest::Summary { kind, sid, .. } => reply(AccountReply::Summary {
            kind,
            to: sid,
            text: summary_text(account, kind),
        }),
    }
}

fn status_text(account: &Account) -> String {
    format!(
        "Account number: {}\nName: {}\nStatus: open and in good standing\n\nNo charges. The Ledger Clerk\nhas nothing to report.",
        account.id.0, account.persona
    )
}

fn summary_text(account: &Account, kind: SummaryKind) -> String {
    let (title, detail) = match kind {
        SummaryKind::Usage => (
            "Usage Summary",
            "Time spent here is not counted.\r\nThe hourglass has been sent on holiday.",
        ),
        SummaryKind::Billing => (
            "Billing Summary",
            "Nothing is billed, and no payment\r\ndetails are kept. The Department of\r\nPaperwork stamped this in triplicate.",
        ),
    };
    format!(
        "{title}\r\n\r\nAccount number: {}\r\n\r\n{detail}\r\n\r\nTotal due: nothing. No charges.\r\n",
        account.id.0
    )
}

/// A 37/30 form: the Ack the map waits for, or the Nak that makes it say the message could not be
/// processed. The Member Services question is kept for the operator; the Accounting form is not.
pub(crate) fn form(world: &World, account: &Account, letter: &OutgoingLetter) -> Vec<MailReply> {
    let outcome = match letter.to {
        to if to == MEMBER_SERVICES_BOX_ASSUMED => member_services(world, account, letter),
        to if to == ACCOUNTING_BOX_ASSUMED => {
            accounting(account, letter);
            Ok(())
        }
        _ => Err(MailFault::NoSuchBox),
    };
    let reply = match outcome {
        Ok(()) => MailReply::Delivered {
            sub: FORM_SUB,
            to: letter.sid,
            mailbox: letter.to,
        },
        Err(fault) => MailReply::Refused {
            sub: FORM_SUB,
            to: letter.sid,
            fault,
            mailbox: letter.to,
        },
    };
    vec![reply]
}

const FORM_SUB: u8 = 30;

fn member_services(
    world: &World,
    account: &Account,
    letter: &OutgoingLetter,
) -> Result<(), MailFault> {
    if !MEMBER_SERVICES_MESSAGES_ARE_KEPT_ASSUMED {
        return Ok(());
    }
    let board = board_id(
        LandType(0),
        BoardRef {
            location: COMPLAINT_BOARD_LOCATION,
            section: MEMBER_SERVICES_SECTION_ASSUMED,
        },
    );
    let body = post_body(account, &letter.envelope.text);
    match crate::quota::may_post(world.store.as_ref(), account.id) {
        Ok(true) => {}
        Ok(false) => return Err(MailFault::NoRoom),
        Err(error) => warn!(%error, "the post quota could not be read"),
    }
    match world.store.post(board, account.id, None, &body) {
        Ok(post) => {
            info!(
                account = account.id.0,
                post = post.0,
                "Member Services message kept"
            );
            Ok(())
        }
        Err(error) => {
            warn!(%error, "Member Services message could not be kept");
            Err(match error {
                StoreError::UnknownAccount(_) | StoreError::UnknownMailbox(_) => {
                    MailFault::PostOfficeClosed
                }
                StoreError::AccountExists(_) | StoreError::Backend(_) => MailFault::HostDisk,
            })
        }
    }
}

/// The form is thrown away unread; only the fact that one arrived is logged.
fn accounting(account: &Account, _letter: &OutgoingLetter) {
    if ACCOUNT_FORMS_ARE_DISCARDED_ASSUMED {
        info!(
            account = account.id.0,
            "account update form received: discarded"
        );
    }
}

/// `author|text`, the shape every post has (docs/server/privacy.md section 1): the author is the
/// stored persona, so the account's export and delete find the message. The client's text ends at
/// its NUL.
fn post_body(account: &Account, text: &[u8]) -> Vec<u8> {
    let own_text = text.split(|&byte| byte == 0).next().unwrap_or_default();
    let own_text = own_text
        .get(..MEMBER_SERVICES_MESSAGE_LIMIT_ASSUMED)
        .unwrap_or(own_text);
    let author = match account.persona.trim() {
        "" => format!("account {}", account.id.0),
        persona => persona.replace('|', "_"),
    };
    let mut body = format!("{author}|Member Services: ").into_bytes();
    body.extend_from_slice(own_text);
    body
}
