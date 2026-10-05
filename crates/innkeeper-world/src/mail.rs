//! The mail host: mailbox assignment and the `EMMsg` requests over the store.
//! The behaviour and its assumptions are described in docs/server/mail.md.

use std::ops::RangeInclusive;

use tracing::{info, warn};

use crate::assumptions::{
    NO_ACCOUNT_MAILBOX_STATUS_ASSUMED, OTHER_BOXES_ARE_PRIVATE_ASSUMED,
    STORE_FAULT_MAILBOX_STATUS_ASSUMED,
};
use crate::message::{StoredLetter, FIELD_LEN};
use crate::{
    Account, AccountId, Envelope, HostMessage, Letter, LetterId, ListedLetter, MailFault,
    MailReply, MailRequest, MailboxAnswer, MailboxNumber, MailboxRequest, OutgoingLetter, Sid,
    StoreError, World,
};

/// The numbers the client accepts as a box: "between 1 and 4294901759" in its own error text.
const BOX_NUMBERS: RangeInclusive<u32> = 1..=0xFFFE_FFFF;
/// The client shows 64 rows of a listing and flags the rest as not shown.
const LISTING_LIMIT: usize = 64;
const MAIL_ICON: u16 = 0x01;
const MAIL_IN_BOX: u16 = 0x02;

/// The answer to command 45: the account's mailbox, assigned on the first request.
pub(crate) fn assign_mailbox(
    world: &World,
    account: &Account,
    request: MailboxRequest,
) -> HostMessage {
    let to = request.sid;
    let answer = match world.store.assign_mailbox(account.id) {
        Ok(mailbox) => MailboxAnswer::Assigned { to, mailbox },
        Err(StoreError::UnknownAccount(_)) => MailboxAnswer::Refused {
            to,
            status: NO_ACCOUNT_MAILBOX_STATUS_ASSUMED,
        },
        Err(error) => {
            warn!(%error, "no mailbox assigned");
            MailboxAnswer::Refused {
                to,
                status: STORE_FAULT_MAILBOX_STATUS_ASSUMED,
            }
        }
    };
    HostMessage::Mailbox(answer)
}

/// The replies to one command 37 request; an account the store does not know gets a refusal.
pub(crate) fn answer(world: &World, account: &Account, request: &MailRequest) -> Vec<HostMessage> {
    let reply = match world.store.assign_mailbox(account.id) {
        Ok(own) => PostOffice {
            world,
            account: account.id,
            own,
        }
        .handle(request),
        Err(error) => request.refused(refusal_of(&error)),
    };
    reply.into_iter().map(HostMessage::Mail).collect()
}

fn refusal_of(error: &StoreError) -> MailFault {
    match error {
        StoreError::UnknownAccount(_) => MailFault::PostOfficeClosed,
        StoreError::UnknownMailbox(_) => MailFault::NoSuchBox,
        StoreError::AccountExists(_) | StoreError::Backend(_) => {
            warn!(%error, "mail request failed in the store");
            MailFault::HostDisk
        }
    }
}

/// One requester's view of the mail: the world and the box that belongs to the requester.
struct PostOffice<'w> {
    world: &'w World,
    account: AccountId,
    own: MailboxNumber,
}

type Outcome = Result<Option<MailReply>, MailFault>;
type Name = [u8; FIELD_LEN];

impl PostOffice<'_> {
    fn handle(&self, request: &MailRequest) -> Option<MailReply> {
        let outcome = match request {
            MailRequest::Boxes { sid, account } => self.boxes(*sid, *account),
            MailRequest::NewMail { sid, mailbox } => self.new_mail(*sid, *mailbox),
            MailRequest::List { sid, mailbox } => self.list(*sid, *mailbox),
            MailRequest::Send { sub, letter } => self.deliver(*sub, letter),
            MailRequest::Form(letter) => {
                info!(to = letter.to.0, "form for a service box dropped");
                Ok(None)
            }
            MailRequest::Read(wanted) => self.read(wanted.sid, wanted.mailbox, wanted.letter),
            MailRequest::Delete(wanted) => self.delete(wanted.mailbox, wanted.letter),
            MailRequest::Forward {
                letter,
                to,
                addressee,
                ..
            } => self.forward(letter.sid, letter.mailbox, letter.letter, *to, *addressee),
            MailRequest::Services { sid, mailbox } => Ok(Some(MailReply::Listing {
                to: *sid,
                mailbox: *mailbox,
                letters: Vec::new(),
            })),
            MailRequest::SystemList { sid } => Ok(Some(MailReply::EmptySystemList { to: *sid })),
        };
        outcome.unwrap_or_else(|fault| request.refused(fault))
    }

    fn boxes(&self, sid: Sid, account: AccountId) -> Outcome {
        match account == self.account {
            true => Ok(Some(MailReply::Boxes {
                to: sid,
                account,
                boxes: vec![self.own],
            })),
            false => Err(MailFault::NotYourBox),
        }
    }

    fn new_mail(&self, sid: Sid, mailbox: MailboxNumber) -> Outcome {
        let mailbox = self.yours(mailbox)?;
        let status = match self.letters(mailbox)?.is_empty() {
            true => 0,
            false => MAIL_ICON | MAIL_IN_BOX,
        };
        Ok(Some(MailReply::NewMail {
            to: sid,
            mailbox,
            status,
        }))
    }

    fn list(&self, sid: Sid, mailbox: MailboxNumber) -> Outcome {
        let mailbox = self.yours(mailbox)?;
        let letters = self.letters(mailbox)?;
        Ok(Some(MailReply::Listing {
            to: sid,
            mailbox,
            letters: letters
                .iter()
                .filter_map(listed)
                .take(LISTING_LIMIT)
                .collect(),
        }))
    }

    fn deliver(&self, sub: u8, letter: &OutgoingLetter) -> Outcome {
        self.put(letter.to, &letter.envelope)?;
        Ok(Some(MailReply::Delivered {
            sub,
            to: letter.sid,
            mailbox: letter.to,
        }))
    }

    fn read(&self, sid: Sid, mailbox: MailboxNumber, id: LetterId) -> Outcome {
        let mailbox = self.yours(mailbox)?;
        let stored = self.find(mailbox, id)?;
        Ok(Some(MailReply::Letter {
            to: sid,
            mailbox,
            id,
            text: stored.envelope.text,
        }))
    }

    fn delete(&self, mailbox: MailboxNumber, id: LetterId) -> Outcome {
        let mailbox = self.yours(mailbox)?;
        match self.world.store.delete_letter(mailbox, id) {
            Ok(true) => Ok(None),
            Ok(false) => Err(MailFault::NoSuchLetter),
            Err(error) => Err(refusal_of(&error)),
        }
    }

    fn forward(
        &self,
        sid: Sid,
        from: MailboxNumber,
        id: LetterId,
        to: MailboxNumber,
        name: Name,
    ) -> Outcome {
        let from = self.yours(from)?;
        let mut envelope = self.find(from, id)?.envelope;
        envelope.addressee = name;
        self.put(to, &envelope)?;
        Ok(Some(MailReply::Forwarded {
            to: sid,
            mailbox: to,
        }))
    }

    /// Only the requester's own box may be read from; other boxes are only written to.
    fn yours(&self, mailbox: MailboxNumber) -> Result<MailboxNumber, MailFault> {
        match !OTHER_BOXES_ARE_PRIVATE_ASSUMED || mailbox == self.own {
            true => Ok(mailbox),
            false => Err(MailFault::NotYourBox),
        }
    }

    fn letters(&self, mailbox: MailboxNumber) -> Result<Vec<Letter>, MailFault> {
        let letters = self.world.store.letters(mailbox);
        letters.map_err(|error| refusal_of(&error))
    }

    fn find(&self, mailbox: MailboxNumber, id: LetterId) -> Result<StoredLetter, MailFault> {
        let letters = self.letters(mailbox)?;
        let letter = letters.iter().find(|letter| letter.id == id);
        let stored = letter.and_then(|letter| stored(letter.id, &letter.body));
        stored.ok_or(MailFault::NoSuchLetter)
    }

    fn put(&self, mailbox: MailboxNumber, envelope: &Envelope) -> Result<(), MailFault> {
        if !BOX_NUMBERS.contains(&mailbox.0) {
            return Err(MailFault::BoxOutOfRange);
        }
        let letter = StoredLetter {
            received: self.world.clock.now(),
            envelope: envelope.clone(),
        };
        let appended = self.world.store.append_letter(mailbox, &letter.to_body());
        appended.map(drop).map_err(|error| refusal_of(&error))
    }
}

fn stored(id: LetterId, body: &[u8]) -> Option<StoredLetter> {
    StoredLetter::from_body(body)
        .inspect_err(|error| warn!(letter = id.0, %error, "stored letter unreadable"))
        .ok()
}

fn listed(letter: &Letter) -> Option<ListedLetter> {
    let StoredLetter { received, envelope } = stored(letter.id, &letter.body)?;
    Some(ListedLetter {
        id: letter.id,
        from: envelope.from,
        subject: envelope.subject,
        sender: envelope.sender,
        flags: envelope.flags,
        received,
        addressee: envelope.addressee,
    })
}
