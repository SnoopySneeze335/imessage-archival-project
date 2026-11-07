//! The invented cast and their conversations.
//!
//! Every person, number and email here is fictional. Phone numbers use the
//! 555-0100..555-0199 range reserved for fiction; emails use example.com,
//! example.org and example.net (RFC 2606).
//!
//! The scenario reproduces, at small scale, every edge case the real
//! backup's diagnostics found (see docs/SPEC.md, "Source data").

use std::collections::BTreeMap;

/// Seconds between 1970-01-01 and Apple's epoch, 2001-01-01 (UTC).
pub const APPLE_EPOCH_OFFSET: i64 = 978_307_200;

/// The fictional phone owner's own addresses.
pub const OWNER_PHONE: &str = "+12065550199";
pub const OWNER_EMAIL: &str = "fixture.owner@example.com";

pub const SERVICE_IMESSAGE: &str = "iMessage";
pub const SERVICE_SMS: &str = "SMS";

pub const STYLE_GROUP: i64 = 43;
pub const STYLE_DIRECT: i64 = 45;

#[derive(Debug, Clone)]
pub struct Handle {
    pub rowid: i64,
    pub id: &'static str,
    pub service: &'static str,
}

#[derive(Debug, Clone)]
pub struct Chat {
    pub rowid: i64,
    pub guid: String,
    pub chat_identifier: String,
    pub service_name: &'static str,
    pub style: i64,
    pub display_name: Option<String>,
    pub participants: Vec<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct Message {
    pub rowid: i64,
    pub guid: String,
    pub text: Option<String>,
    pub handle_id: i64,
    pub service: &'static str,
    /// Unix seconds; converted to Apple-epoch nanoseconds on write.
    pub date: i64,
    pub date_read: i64,
    pub date_delivered: i64,
    pub date_edited: i64,
    pub date_retracted: i64,
    pub is_from_me: bool,
    pub is_read: bool,
    pub is_delivered: bool,
    pub is_audio_message: bool,
    pub is_system_message: bool,
    pub item_type: i64,
    pub group_action_type: i64,
    pub other_handle: i64,
    pub group_title: Option<String>,
    pub associated_message_guid: Option<String>,
    pub associated_message_type: i64,
    pub associated_message_emoji: Option<String>,
    pub associated_message_range_length: i64,
    pub thread_originator_guid: Option<String>,
    pub thread_originator_part: Option<String>,
    pub attachments: Vec<i64>,
    /// Chat the message belongs to; `None` for the orphan case.
    pub chat: Option<i64>,
    /// Deleted on the phone but still recoverable (iOS 16+).
    pub recoverable_in: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Attachment {
    pub rowid: i64,
    pub guid: String,
    /// Path as sms.db stores it (`~/Library/SMS/Attachments/...`); `None`
    /// reproduces the "no path" case.
    pub filename: Option<String>,
    pub transfer_name: String,
    pub mime_type: &'static str,
    pub uti: &'static str,
    pub total_bytes: i64,
    pub is_outgoing: bool,
    pub is_sticker: bool,
    /// Unix seconds.
    pub created: i64,
    /// File bytes; `None` means the backup does not contain the file.
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct Contact {
    pub first: Option<&'static str>,
    pub last: Option<&'static str>,
    pub organization: Option<&'static str>,
    /// Phone numbers as typed into the address book (deliberately mixed formats).
    pub phones: Vec<&'static str>,
    pub emails: Vec<&'static str>,
}

#[derive(Debug, Default)]
pub struct Scenario {
    pub handles: Vec<Handle>,
    pub chats: Vec<Chat>,
    pub messages: Vec<Message>,
    pub attachments: Vec<Attachment>,
    pub contacts: Vec<Contact>,
}

/// Unix seconds for a UTC date and time.
pub fn utc(y: i64, m: i64, d: i64, hh: i64, mm: i64) -> i64 {
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + hh * 3_600 + mm * 60
}

/// Unix seconds → Apple-epoch nanoseconds (0 stays 0, meaning "unset").
pub fn apple_ns(unix: i64) -> i64 {
    if unix == 0 {
        0
    } else {
        (unix - APPLE_EPOCH_OFFSET) * 1_000_000_000
    }
}

/// Unix seconds → Apple-epoch seconds.
pub fn apple_s(unix: i64) -> i64 {
    unix - APPLE_EPOCH_OFFSET
}

fn guid(prefix: u32, n: usize) -> String {
    format!("{prefix:08X}-0000-4000-8000-{n:012X}")
}

/// Sender of a message.
#[derive(Debug, Clone, Copy)]
enum From {
    Me,
    H(i64),
}

/// Classic tapbacks (associated_message_type 2000..2005) and the iOS 17+
/// emoji reaction (2006). Removing one adds 1000.
#[derive(Debug, Clone, Copy)]
enum Tapback {
    Loved = 2000,
    Liked = 2001,
    Laughed = 2003,
    Emphasized = 2004,
    Questioned = 2005,
    Emoji = 2006,
}

impl Tapback {
    fn verb(self) -> &'static str {
        match self {
            Tapback::Loved => "Loved",
            Tapback::Liked => "Liked",
            Tapback::Laughed => "Laughed at",
            Tapback::Emphasized => "Emphasized",
            Tapback::Questioned => "Questioned",
            Tapback::Emoji => "Reacted",
        }
    }
}

struct Builder {
    s: Scenario,
    now: i64,
}

impl Builder {
    fn at(&mut self, y: i64, m: i64, d: i64, hh: i64, mm: i64) -> &mut Self {
        self.now = utc(y, m, d, hh, mm);
        self
    }

    fn wait(&mut self, minutes: i64) -> &mut Self {
        self.now += minutes * 60;
        self
    }

    fn chat(&self, rowid: i64) -> &Chat {
        &self.s.chats[(rowid - 1) as usize]
    }

    fn push(&mut self, chat: Option<i64>, from: From, text: Option<String>) -> usize {
        let rowid = self.s.messages.len() as i64 + 1;
        let (service, direct_handle) = match chat {
            Some(c) => {
                let chat = self.chat(c);
                let direct = (chat.style == STYLE_DIRECT).then(|| chat.participants[0]);
                (chat.service_name, direct)
            }
            None => (SERVICE_IMESSAGE, None),
        };
        let (is_from_me, handle_id) = match from {
            // In 1:1 chats iOS stores the other party's handle even on sent messages.
            From::Me => (true, direct_handle.unwrap_or(0)),
            From::H(h) => (false, h),
        };
        let m = Message {
            rowid,
            guid: guid(0xF1C7_0000, rowid as usize),
            text,
            handle_id,
            service,
            date: self.now,
            date_delivered: if is_from_me { self.now + 2 } else { 0 },
            date_read: self.now + 60,
            is_from_me,
            is_read: true,
            is_delivered: true,
            chat,
            ..Message::default()
        };
        self.s.messages.push(m);
        self.now += 45;
        self.s.messages.len() - 1
    }

    fn say(&mut self, chat: i64, from: From, text: &str) -> String {
        let i = self.push(Some(chat), from, Some(text.to_owned()));
        self.s.messages[i].guid.clone()
    }

    fn react(&mut self, chat: i64, from: From, target: &str, kind: Tapback, remove: bool) {
        let target_text = self
            .s
            .messages
            .iter()
            .find(|m| m.guid == target)
            .and_then(|m| m.text.clone())
            .unwrap_or_default();
        let emoji = matches!(kind, Tapback::Emoji).then(|| "🥾".to_owned());
        let verb = match (&emoji, remove) {
            (Some(e), false) => format!("Reacted {e} to"),
            (Some(e), true) => format!("Removed a {e} from"),
            (None, false) => kind.verb().to_owned(),
            (None, true) => format!("Removed a {} from", kind.verb().to_lowercase()),
        };
        let text = format!("{verb} “{target_text}”");
        let i = self.push(Some(chat), from, Some(text));
        let m = &mut self.s.messages[i];
        m.associated_message_guid = Some(format!("p:0/{target}"));
        m.associated_message_type = kind as i64 + if remove { 1000 } else { 0 };
        m.associated_message_emoji = emoji;
        m.associated_message_range_length = target_text.encode_utf16().count() as i64;
    }

    fn reply(&mut self, chat: i64, from: From, target: &str, text: &str) -> String {
        let len = self
            .s
            .messages
            .iter()
            .find(|m| m.guid == target)
            .and_then(|m| m.text.as_ref())
            .map_or(0, |t| t.encode_utf16().count());
        let i = self.push(Some(chat), from, Some(text.to_owned()));
        let m = &mut self.s.messages[i];
        m.thread_originator_guid = Some(target.to_owned());
        m.thread_originator_part = Some(format!("0:0:{len}"));
        m.guid.clone()
    }

    fn system(
        &mut self,
        chat: i64,
        from: From,
        item_type: i64,
        action: i64,
        other: i64,
        title: Option<&str>,
    ) {
        let i = self.push(Some(chat), from, None);
        let m = &mut self.s.messages[i];
        m.item_type = item_type;
        m.group_action_type = action;
        m.other_handle = other;
        m.group_title = title.map(str::to_owned);
        m.is_system_message = false; // iOS leaves this 0 for group events
    }

    #[allow(clippy::too_many_arguments)]
    fn attach(
        &mut self,
        chat: i64,
        from: From,
        name: &str,
        mime: &'static str,
        uti: &'static str,
        content: Option<Vec<u8>>,
        with_path: bool,
        sticker: bool,
    ) -> usize {
        let i = self.push(Some(chat), from, Some("\u{FFFC}".to_owned()));
        let arowid = self.s.attachments.len() as i64 + 1;
        let aguid = guid(0xA77A_0000, arowid as usize);
        let dir = format!("{:02x}/{:02}", (arowid * 37) % 256, arowid % 100);
        let filename = with_path.then(|| format!("~/Library/SMS/Attachments/{dir}/{aguid}/{name}"));
        let total_bytes = content.as_ref().map_or(48_213, |c| c.len() as i64);
        let is_outgoing = matches!(from, From::Me);
        self.s.attachments.push(Attachment {
            rowid: arowid,
            guid: aguid,
            filename,
            transfer_name: name.to_owned(),
            mime_type: mime,
            uti,
            total_bytes,
            is_outgoing,
            is_sticker: sticker,
            created: self.now,
            content,
        });
        self.s.messages[i].attachments.push(arowid);
        i
    }
}

/// A tiny invented HEIC image (see fixtures/media/README.md).
const HEIC_SAMPLE: &[u8] = include_bytes!("../../media/fixture-32x32.heic");

const FILLER: &[&str] = &[
    "Did you see the new trail map?",
    "Running ten minutes late, sorry",
    "Can you grab oat milk on the way?",
    "That ferry schedule is wild",
    "Lunch at the noodle place?",
    "I finally fixed the bike chain",
    "Haha no way",
    "Sounds good to me",
    "What time does the game start?",
    "Remind me to call the landlord",
    "The rain stopped, heading out now",
    "Sending the photos tonight",
    "Who has the spare key?",
    "Board game night on Friday?",
    "OK",
    "Thanks!!",
    "Brb",
    "The bakery was closed again",
    "Do we need a reservation?",
    "Happy birthday!! 🎂",
];

/// Builds the whole scenario. Pure function: same output every call.
pub fn build() -> Scenario {
    let handles = [
        ("+12065550101", SERVICE_IMESSAGE),                // 1 Avery
        ("+12065550101", SERVICE_SMS),                     // 2 Avery, same number via SMS
        ("avery.quill@example.com", SERVICE_IMESSAGE),     // 3 Avery, email
        ("+12065550102", SERVICE_IMESSAGE),                // 4 Bram
        ("+12065550103", SERVICE_IMESSAGE),                // 5 Cleo
        ("+12065550104", SERVICE_IMESSAGE),                // 6 Dmitri
        ("+12065550105", SERVICE_IMESSAGE),                // 7 Esme
        ("esme.nakagawa@example.org", SERVICE_IMESSAGE),   // 8 Esme, email, never used
        ("+12065550150", SERVICE_SMS),                     // 9 unknown
        ("+12065550151", SERVICE_IMESSAGE),                // 10 unknown
        ("55501", SERVICE_SMS),                            // 11 short code
        ("someone.unknown@example.net", SERVICE_IMESSAGE), // 12 unknown email
        ("+12065550170", SERVICE_SMS),                     // 13 business in address book
    ];
    let handles = handles
        .iter()
        .enumerate()
        .map(|(i, &(id, service))| Handle {
            rowid: i as i64 + 1,
            id,
            service,
        })
        .collect();

    let direct = |rowid: i64, prefix: &str, ident: &str, service: &'static str, h: i64| Chat {
        rowid,
        guid: format!("{prefix};-;{ident}"),
        chat_identifier: ident.to_owned(),
        service_name: service,
        style: STYLE_DIRECT,
        display_name: None,
        participants: vec![h],
    };
    let chats = vec![
        direct(1, "iMessage", "+12065550101", SERVICE_IMESSAGE, 1),
        direct(2, "SMS", "+12065550101", SERVICE_SMS, 2),
        direct(
            3,
            "iMessage",
            "avery.quill@example.com",
            SERVICE_IMESSAGE,
            3,
        ),
        direct(4, "iMessage", "+12065550102", SERVICE_IMESSAGE, 4),
        // Duplicate of chat 4: same identifier, different GUID.
        direct(5, "any", "+12065550102", SERVICE_IMESSAGE, 4),
        Chat {
            rowid: 6,
            guid: "iMessage;+;chat100000000000000001".to_owned(),
            chat_identifier: "chat100000000000000001".to_owned(),
            service_name: SERVICE_IMESSAGE,
            style: STYLE_GROUP,
            display_name: Some("Trail Crew 🥾".to_owned()),
            // Current members; Dmitri (6) left, Esme (7) joined.
            participants: vec![4, 5, 7],
        },
        direct(7, "SMS", "+12065550150", SERVICE_SMS, 9),
        direct(8, "iMessage", "+12065550151", SERVICE_IMESSAGE, 10),
        direct(9, "SMS", "55501", SERVICE_SMS, 11),
        direct(
            10,
            "iMessage",
            "someone.unknown@example.net",
            SERVICE_IMESSAGE,
            12,
        ),
        direct(11, "iMessage", "+12065550105", SERVICE_IMESSAGE, 7),
        direct(12, "SMS", "+12065550170", SERVICE_SMS, 13),
    ];

    let contacts = vec![
        Contact {
            first: Some("Avery"),
            last: Some("Quill"),
            organization: None,
            phones: vec!["(206) 555-0101"],
            emails: vec!["Avery.Quill@example.com"],
        },
        Contact {
            first: Some("Bram"),
            last: Some("Okonkwo"),
            organization: None,
            phones: vec!["206-555-0102"],
            emails: vec![],
        },
        Contact {
            first: Some("Cleo"),
            last: Some("Marchetti"),
            organization: None,
            phones: vec!["+1 206 555 0103"],
            emails: vec![],
        },
        Contact {
            first: Some("Dmitri"),
            last: Some("Vale"),
            organization: None,
            phones: vec!["2065550104"],
            emails: vec![],
        },
        Contact {
            first: Some("Esme"),
            last: Some("Nakagawa"),
            organization: None,
            phones: vec!["206.555.0105"],
            emails: vec!["esme.nakagawa@example.org"],
        },
        // In the address book but never messaged.
        Contact {
            first: Some("Gus"),
            last: Some("Fairweather"),
            organization: None,
            phones: vec!["(206) 555-0160"],
            emails: vec![],
        },
        // A business entry.
        Contact {
            first: None,
            last: None,
            organization: Some("Northwind Pharmacy"),
            phones: vec!["(206) 555-0170"],
            emails: vec![],
        },
    ];

    let mut b = Builder {
        s: Scenario {
            handles,
            chats,
            contacts,
            ..Scenario::default()
        },
        now: 0,
    };
    let (avery, avery_sms, avery_mail, bram, cleo, dmitri, esme) = (
        From::H(1),
        From::H(2),
        From::H(3),
        From::H(4),
        From::H(5),
        From::H(6),
        From::H(7),
    );
    let me = From::Me;

    // Chat 1: Avery over iMessage. Reactions, replies, edits, unsend, attachments.
    b.at(2023, 6, 3, 16, 5);
    let hey = b.say(1, avery, "Hey! Are we still on for the café on Saturday?");
    let yes = b.say(
        1,
        me,
        "Yes — 10am works. I need to recieve a package first.",
    );
    b.react(1, avery, &yes, Tapback::Loved, false);
    b.reply(1, avery, &hey, "Perfect, I'll grab a table by the window");
    let edited = b.say(1, me, "See you at 10:30 instead");
    {
        let m = b.s.messages.iter_mut().find(|m| m.guid == edited).unwrap();
        m.date_edited = m.date + 90;
    }
    let unsent = b.push(Some(1), me, None);
    b.s.messages[unsent].date_retracted = b.s.messages[unsent].date + 30;
    b.s.messages[unsent].date_edited = b.s.messages[unsent].date + 30;
    b.react(1, me, &hey, Tapback::Laughed, false);
    b.react(1, avery, &yes, Tapback::Loved, true);
    b.react(1, avery, &yes, Tapback::Liked, false);
    b.wait(180);
    b.attach(
        1,
        avery,
        "IMG_0001.PNG",
        "image/png",
        "public.png",
        Some(crate::media::png(16, [242, 140, 40], [40, 120, 242])),
        true,
        false,
    );
    b.attach(
        1,
        avery,
        "IMG_0002.HEIC",
        "image/heic",
        "public.heic",
        None,
        true,
        false,
    ); // file missing
    b.attach(
        1,
        me,
        "IMG_0003.JPG",
        "image/jpeg",
        "public.jpeg",
        None,
        false,
        false,
    ); // no path
    b.say(1, avery, &long_message());
    b.wait(60 * 24 * 40);
    let deleted = b.push(
        None,
        avery,
        Some("This one was deleted but is still recoverable".to_owned()),
    );
    b.s.messages[deleted].recoverable_in = Some(1);
    b.say(
        1,
        me,
        "Naïve question: does the façade still have that mural?",
    );

    // Chat 2: Avery, same number, over SMS (green bubbles).
    b.at(2023, 9, 14, 21, 30);
    b.say(2, avery_sms, "No data here, texting instead");
    b.say(2, me, "Got it. Call when you land?");
    let late = b.push(Some(2), me, Some("Delivered but never read".to_owned()));
    b.s.messages[late].is_read = false;
    b.s.messages[late].date_read = 0;

    // Chat 3: Avery's email address.
    b.at(2024, 1, 2, 8, 15);
    b.say(3, avery_mail, "Sending from my laptop — happy new year!");
    b.say(3, me, "Happy new year!! 🎉");

    // Chat 4: Bram, long thread of filler across 2024 for windowing and stats.
    b.at(2024, 2, 1, 7, 0);
    for i in 0..150usize {
        let from = if (i * 7 + 3) % 5 < 2 { me } else { bram };
        let text = FILLER[(i * 7) % FILLER.len()];
        b.say(4, from, text);
        // Mixed gaps: seconds-apart runs plus hour and day breaks.
        b.wait(match i % 9 {
            0 => 60 * 26,
            4 => 95,
            7 => 60 * 5,
            _ => 1,
        });
    }
    // Chat 5: duplicate of chat 4.
    b.at(2024, 8, 9, 12, 0);
    b.say(5, bram, "New phone, who dis 😄");
    b.say(5, me, "lol");

    // Chat 6: group with renames, joins and leaves.
    b.at(2024, 3, 9, 18, 0);
    b.system(6, me, 2, 0, 0, Some("Trail Crew"));
    b.say(6, bram, "Saturday hike: Rattlesnake Ledge?");
    let q = b.say(6, cleo, "I'm in if we start before 9");
    b.say(6, dmitri, "Can't this week, next time");
    b.react(6, me, &q, Tapback::Emphasized, false);
    b.react(6, bram, &q, Tapback::Questioned, false);
    b.system(6, bram, 1, 0, 7, None); // Bram added Esme
    b.say(6, esme, "Thanks for adding me!");
    b.react(6, esme, &q, Tapback::Emoji, false);
    b.attach(
        6,
        cleo,
        "IMG_0004.PNG",
        "image/png",
        "public.png",
        Some(crate::media::png(24, [60, 160, 90], [230, 230, 230])),
        true,
        false,
    );
    b.system(6, cleo, 2, 0, 0, Some("Trail Crew 🥾"));
    b.system(6, dmitri, 1, 1, 6, None); // Dmitri left
    b.say(6, me, "Summit photo coming soon");

    // Chats 7–10: unknown numbers, a short code, an unknown email.
    b.at(2024, 5, 20, 14, 10);
    b.say(
        7,
        From::H(9),
        "Hi, is the bike from the listing still available?",
    );
    b.say(7, me, "Yes! Want to see it tomorrow?");
    b.say(7, From::H(9), "Great, 5pm works");
    b.at(2024, 6, 1, 10, 0);
    b.say(
        8,
        From::H(10),
        "Hey it's Jo from the climbing gym, new number",
    );
    b.at(2024, 6, 2, 3, 12);
    for code in ["000000", "000001", "000002"] {
        b.say(
            9,
            From::H(11),
            &format!("Your Fixturebank code is {code}. Do not share it."),
        );
        b.wait(60 * 24 * 3);
    }
    b.at(2024, 7, 4, 9, 0);
    b.say(
        10,
        From::H(12),
        "Your package is waiting at the front desk.",
    );

    // Chat 11: Esme. Voice message, sticker, image.
    b.at(2025, 2, 14, 19, 45);
    b.say(11, esme, "Listen to this 🎵");
    let v = b.attach(
        11,
        esme,
        "Audio Message.caf",
        "audio/x-caf",
        "com.apple.coreaudio-format",
        Some(crate::media::caf_voice(8_000, 4_000)),
        true,
        false,
    );
    b.s.messages[v].is_audio_message = true;
    b.attach(
        11,
        me,
        "sticker.png",
        "image/png",
        "public.png",
        Some(crate::media::png(8, [255, 210, 0], [255, 255, 255])),
        true,
        true,
    );
    b.attach(
        11,
        me,
        "IMG_0005.PNG",
        "image/png",
        "public.png",
        Some(crate::media::png(32, [20, 20, 80], [200, 200, 255])),
        true,
        false,
    );
    b.attach(
        11,
        esme,
        "IMG_0006.HEIC",
        "image/heic",
        "public.heic",
        Some(HEIC_SAMPLE.to_vec()),
        true,
        false,
    );
    b.say(11, me, "Résumé attached earlier, let me know!");

    // Chat 12: the business from the address book.
    b.at(2025, 3, 3, 16, 0);
    b.say(
        12,
        From::H(13),
        "Northwind Pharmacy: your order is ready for pickup.",
    );
    b.say(12, me, "Thanks");

    // A message linked to no chat at all.
    b.at(2025, 4, 1, 12, 0);
    b.push(
        None,
        From::H(10),
        Some("Orphaned message with no chat".to_owned()),
    );

    b.s
}

fn long_message() -> String {
    let words = [
        "the", "ridge", "trail", "lantern", "harbor", "maple", "quiet", "river", "north", "window",
    ];
    let mut out = String::from("Long message: ");
    for i in 0..120 {
        out.push_str(words[(i * 3 + i / 7) % words.len()]);
        out.push(if i % 15 == 14 { '.' } else { ' ' });
    }
    out.trim_end().to_owned()
}

/// Message count per chat GUID (messages in chat_message_join only).
pub fn per_chat_counts(s: &Scenario) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for chat in &s.chats {
        out.insert(
            chat.guid.clone(),
            s.messages
                .iter()
                .filter(|m| m.chat == Some(chat.rowid))
                .count(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_matches_known_timestamps() {
        assert_eq!(utc(1970, 1, 1, 0, 0), 0);
        assert_eq!(utc(2001, 1, 1, 0, 0), APPLE_EPOCH_OFFSET);
        assert_eq!(utc(2026, 1, 4, 0, 0), 1_767_484_800);
    }

    #[test]
    fn numbers_are_fictional() {
        let s = build();
        for h in &s.handles {
            if let Some(rest) = h.id.strip_prefix("+1206555") {
                assert!(rest.starts_with("01") && rest.len() == 4, "{}", h.id);
            } else if h.id.contains('@') {
                assert!(
                    [
                        ".example.com",
                        "@example.com",
                        "@example.org",
                        "@example.net"
                    ]
                    .iter()
                    .any(|d| h.id.ends_with(d)),
                    "{}",
                    h.id
                );
            } else {
                assert_eq!(h.id, "55501");
            }
        }
    }
}
