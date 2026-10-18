//! The mail host: mailbox assignment and the `EMMsg` requests over the store.
//! The behaviour and its assumptions are described in docs/server/mail.md.

use std::ops::RangeInclusive;

use tracing::{info, warn};

use crate::assumptions::{
    MAILBOX_LETTERS_MAX_ASSUMED, NO_ACCOUNT_MAILBOX_STATUS_ASSUMED,
    OTHER_BOXES_ARE_PRIVATE_ASSUMED, SHORT_MAIL_FORM_IS_REFUSED_ASSUMED,
    STORE_FAULT_MAILBOX_STATUS_ASSUMED,
};
use crate::message::{StoredLetter, FIELD_LEN};
use crate::{quota, town_hall};
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
    let answer = match own_mailbox(world, account.id) {
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
    // The system list and the service forms need no mailbox of the requester's, so an open book
    // (no stored accounts) answers them too; the Town Hall works without a data directory.
    let replies = match request {
        MailRequest::SystemList { sid } => Some(vec![MailReply::SystemList {
            to: *sid,
            services: town_hall::system_list(),
        }]),
        MailRequest::Form(letter) => Some(town_hall::form(world, account, letter)),
        _ => None,
    };
    if let Some(replies) = replies {
        return replies.into_iter().map(HostMessage::Mail).collect();
    }
    let reply = match own_mailbox(world, account.id) {
        Ok(own) => PostOffice {
            world,
            account: account.id,
            persona: &account.persona,
            own,
        }
        .handle(request),
        Err(error) => request.refused(refusal_of(&error)),
    };
    reply.into_iter().map(HostMessage::Mail).collect()
}

/// The account's box. A book that is not the store's (the open book) runs no post office: its
/// accounts have store rows only so their other writes have a parent (`logon::give_a_row`), and the
/// client is told there is no mailbox, as before those rows existed (mail.md section 6).
fn own_mailbox(world: &World, account: AccountId) -> Result<MailboxNumber, StoreError> {
    match world.accounts.is_stored() {
        true => world.store.assign_mailbox(account),
        false => Err(StoreError::UnknownAccount(account)),
    }
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
    /// The session's persona: what a letter it sends says it is from.
    persona: &'w str,
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
            MailRequest::Form(_) | MailRequest::SystemList { .. } => Ok(None),
            MailRequest::Page { .. } => self.page(),
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
        };
        outcome.unwrap_or_else(|fault| request.refused(fault))
    }

    /// The sysop pages (the short 37/20 and 37/21) are refused: nobody here has sysop rights over a
    /// box (`SHORT_MAIL_FORM_IS_REFUSED_ASSUMED`).
    fn page(&self) -> Outcome {
        match SHORT_MAIL_FORM_IS_REFUSED_ASSUMED {
            true => Err(MailFault::NotYourBox),
            false => Ok(None),
        }
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
        let held = self.world.store.letter_count(mailbox);
        let status = match held.map_err(|error| refusal_of(&error))? {
            0 => 0,
            _ => MAIL_ICON | MAIL_IN_BOX,
        };
        Ok(Some(MailReply::NewMail {
            to: sid,
            mailbox,
            status,
        }))
    }

    fn list(&self, sid: Sid, mailbox: MailboxNumber) -> Outcome {
        let mailbox = self.yours(mailbox)?;
        let letters = self.world.store.first_letters(mailbox, LISTING_LIMIT);
        let letters = letters.map_err(|error| refusal_of(&error))?;
        Ok(Some(MailReply::Listing {
            to: sid,
            mailbox,
            letters: letters.iter().filter_map(listed).collect(),
        }))
    }

    fn deliver(&self, sub: u8, letter: &OutgoingLetter) -> Outcome {
        self.put(letter.to, &self.stamped(&letter.envelope))?;
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

    /// The envelope with the sender's own box and the session's persona in its `from` fields, so a
    /// letter cannot pass for another player's or the staff's. An honest client's bytes stay as sent.
    fn stamped(&self, envelope: &Envelope) -> Envelope {
        let mut stamped = envelope.clone();
        if stamped.from != self.own {
            info!(
                account = self.account.0,
                "letter named a box that is not the sender's: replaced"
            );
            stamped.from = self.own;
        }
        let persona = name_field(self.persona);
        if field_text(&stamped.sender) != field_text(&persona) {
            info!(
                account = self.account.0,
                "letter named a sender that is not the session's persona: replaced"
            );
            stamped.sender = persona;
        }
        stamped
    }

    /// Only the requester's own box may be read from; other boxes are only written to.
    fn yours(&self, mailbox: MailboxNumber) -> Result<MailboxNumber, MailFault> {
        match !OTHER_BOXES_ARE_PRIVATE_ASSUMED || mailbox == self.own {
            true => Ok(mailbox),
            false => Err(MailFault::NotYourBox),
        }
    }

    fn find(&self, mailbox: MailboxNumber, id: LetterId) -> Result<StoredLetter, MailFault> {
        let letter = self.world.store.letter(mailbox, id);
        let letter = letter.map_err(|error| refusal_of(&error))?;
        let stored = letter.and_then(|letter| stored(letter.id, &letter.body));
        stored.ok_or(MailFault::NoSuchLetter)
    }

    /// A send within the sender's daily quota, to a box with room for it.
    fn has_room(&self, mailbox: MailboxNumber) -> Result<(), MailFault> {
        let store = self.world.store.as_ref();
        let fault = |error: StoreError| refusal_of(&error);
        if !quota::may_send_letter(store, self.account).map_err(fault)? {
            return Err(MailFault::NoRoom);
        }
        let held = store.letter_count(mailbox).map_err(fault)?;
        if held >= MAILBOX_LETTERS_MAX_ASSUMED {
            warn!(mailbox = mailbox.0, held, "letter refused: the box is full");
            return Err(MailFault::NoRoom);
        }
        Ok(())
    }

    fn put(&self, mailbox: MailboxNumber, envelope: &Envelope) -> Result<(), MailFault> {
        if !BOX_NUMBERS.contains(&mailbox.0) {
            return Err(MailFault::BoxOutOfRange);
        }
        self.has_room(mailbox)?;
        let letter = StoredLetter {
            received: self.world.clock.now(),
            envelope: envelope.clone(),
        };
        let appended = self
            .world
            .store
            .append_letter(mailbox, self.account, &letter.to_body());
        appended.map(drop).map_err(|error| refusal_of(&error))
    }
}

/// A persona as the client's 10-byte name field: cut to 10 bytes, NUL padded.
fn name_field(persona: &str) -> Name {
    let mut field = [0u8; FIELD_LEN];
    for (slot, byte) in field.iter_mut().zip(persona.bytes()) {
        *slot = byte;
    }
    field
}

/// A name field's text: the bytes before the first NUL.
fn field_text(field: &Name) -> &[u8] {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(FIELD_LEN);
    field.get(..end).unwrap_or(field)
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
