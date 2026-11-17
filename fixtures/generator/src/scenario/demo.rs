//! The demo backup: the v1 scenario plus two "All message kinds"
//! conversations, a 1:1 and a group, that hold one example of every kind
//! and state the viewer knows, in a fixed order. Each example follows a
//! label message from the owner, "▶ <label>: <what it shows>", naming the
//! view label (`imessage_ap::view::FEATURES`) the example must carry. A test
//! in `crates/imessage-ap/tests/demo.rs` checks both.
//!
//! With `large`, a third conversation holds [`LARGE_CHAT_MESSAGES`] messages
//! over six years, for scrolling.
//!
//! Every person, number and address here is invented.

use super::*;

/// Messages in the large conversation.
pub const LARGE_CHAT_MESSAGES: usize = 25_000;

/// Title of the two coverage conversations.
pub const KINDS_TITLE: &str = "All message kinds";
pub const KINDS_GROUP_TITLE: &str = "All message kinds (group)";

const ROWAN: i64 = 14;
const THEO: i64 = 15;
const KINDS_CHAT: i64 = 13;
const KINDS_GROUP: i64 = 14;
/// A chat no message belongs to, and one whose only messages are reactions:
/// both exist in real backups and are not listed.
const EMPTY_CHAT: i64 = 15;
const REACTIONS_ONLY_CHAT: i64 = 16;
const LARGE_CHAT: i64 = 17;

const APP_BALLOON: &str = "com.apple.messages.MSMessageExtensionBalloonPlugin:EXAMPLE001:com.example.fixturegame.MessagesExtension";

/// A tiny invented PDF (one blank page).
const PDF: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 72 72]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";

impl Builder {
    /// The owner's label message before an example.
    fn label(&mut self, chat: i64, feature: &str, what: &str) {
        self.wait(2);
        self.say(chat, From::Me, &format!("▶ {feature}: {what}"));
    }

    /// Adds an attachment to an existing message.
    #[allow(clippy::too_many_arguments)]
    fn add_file(
        &mut self,
        i: usize,
        name: &str,
        mime: &'static str,
        uti: &'static str,
        content: Option<Vec<u8>>,
        with_path: bool,
        sticker: bool,
    ) -> i64 {
        let from = if self.s.messages[i].is_from_me {
            From::Me
        } else {
            From::H(self.s.messages[i].handle_id)
        };
        let a = self.attachment(name, mime, uti, content, with_path, sticker, from);
        self.s.messages[i].attachments.push(a);
        a
    }
}

fn png(n: u8) -> Vec<u8> {
    crate::media::png(
        16 + u32::from(n) * 4,
        [
            40u8.wrapping_add(n.wrapping_mul(25)),
            90,
            200u8.wrapping_sub(n.wrapping_mul(20)),
        ],
        [240, 240, 240],
    )
}

/// The v1 scenario plus the demo conversations.
pub fn build_demo(large: bool) -> Scenario {
    let mut b = Builder { s: build(), now: 0 };
    b.s.handles.push(Handle {
        rowid: ROWAN,
        id: "+12065550120",
        service: SERVICE_IMESSAGE,
    });
    b.s.handles.push(Handle {
        rowid: THEO,
        id: "+12065550121",
        service: SERVICE_IMESSAGE,
    });
    b.s.contacts.push(Contact {
        first: Some("Rowan"),
        last: Some("Pike"),
        organization: None,
        phones: vec!["(206) 555-0120"],
        emails: vec![],
    });
    b.s.contacts.push(Contact {
        first: Some("Theo"),
        last: Some("Lindqvist"),
        organization: None,
        phones: vec!["206-555-0121"],
        emails: vec![],
    });
    b.s.chats.push(Chat {
        rowid: KINDS_CHAT,
        guid: "iMessage;-;+12065550120".to_owned(),
        chat_identifier: "+12065550120".to_owned(),
        service_name: SERVICE_IMESSAGE,
        style: STYLE_DIRECT,
        display_name: Some(KINDS_TITLE.to_owned()),
        participants: vec![ROWAN],
    });
    b.s.chats.push(Chat {
        rowid: KINDS_GROUP,
        guid: "iMessage;+;chat100000000000000002".to_owned(),
        chat_identifier: "chat100000000000000002".to_owned(),
        service_name: SERVICE_IMESSAGE,
        style: STYLE_GROUP,
        display_name: Some(KINDS_GROUP_TITLE.to_owned()),
        participants: vec![ROWAN, 1, 4],
    });
    b.s.chats.push(Chat {
        rowid: EMPTY_CHAT,
        guid: "SMS;-;+12065550121".to_owned(),
        chat_identifier: "+12065550121".to_owned(),
        service_name: SERVICE_SMS,
        style: STYLE_DIRECT,
        display_name: None,
        participants: vec![THEO],
    });
    b.s.chats.push(Chat {
        rowid: REACTIONS_ONLY_CHAT,
        guid: "SMS;-;+12065550120".to_owned(),
        chat_identifier: "+12065550120".to_owned(),
        service_name: SERVICE_SMS,
        style: STYLE_DIRECT,
        display_name: None,
        participants: vec![ROWAN],
    });
    kinds_chat(&mut b);
    kinds_group(&mut b);
    if large {
        b.s.chats.push(Chat {
            rowid: LARGE_CHAT,
            guid: "iMessage;-;+12065550121".to_owned(),
            chat_identifier: "+12065550121".to_owned(),
            service_name: SERVICE_IMESSAGE,
            style: STYLE_DIRECT,
            display_name: None,
            participants: vec![THEO],
        });
        large_chat(&mut b);
    }
    b.s
}

fn kinds_chat(b: &mut Builder) {
    let (c, me, rowan) = (KINDS_CHAT, From::Me, From::H(ROWAN));
    b.at(2025, 4, 20, 17, 0);
    b.say(
        c,
        me,
        "This conversation shows one example of every kind of message, in order.",
    );

    b.label(c, "text", "plain text");
    let first = b.say(c, rowan, "Plain text from Rowan. Nothing special about it.");

    b.label(c, "subject", "a subject line above the text");
    let i = b.push(
        Some(c),
        rowan,
        Some("Bring the map and the blue thermos.".into()),
    );
    b.s.messages[i].subject = Some("Saturday hike".into());

    b.label(c, "emoji_only", "a few emoji, drawn large");
    b.say(c, rowan, "🎉🥾");

    b.label(c, "media:web_image", "a PNG, shown as it is");
    b.attach(
        c,
        rowan,
        "IMG_2001.PNG",
        "image/png",
        "public.png",
        Some(png(1)),
        true,
        false,
    );

    b.label(c, "attachments_multiple", "three photos in one message");
    let i = b.push(Some(c), me, Some("\u{FFFC}\u{FFFC}\u{FFFC}".into()));
    for n in 0..3u8 {
        b.add_file(
            i,
            &format!("IMG_21{n:02}.PNG"),
            "image/png",
            "public.png",
            Some(png(2 + n)),
            true,
            false,
        );
    }

    b.label(c, "media:image", "a HEIC photo, shown through its preview");
    b.attach(
        c,
        rowan,
        "IMG_2201.HEIC",
        "image/heic",
        "public.heic",
        Some(HEIC_SAMPLE.to_vec()),
        true,
        false,
    );

    b.label(c, "media:video", "a video");
    b.attach(
        c,
        me,
        "IMG_2301.MP4",
        "video/mp4",
        "public.mpeg-4",
        Some(AAC_VIDEO_SAMPLE.to_vec()),
        true,
        false,
    );

    b.label(c, "audio_message", "a voice message");
    let v = b.attach(
        c,
        rowan,
        "Audio Message.caf",
        "audio/x-caf",
        "com.apple.coreaudio-format",
        Some(crate::media::caf_aac(AAC_AUDIO_SAMPLE)),
        true,
        false,
    );
    b.s.messages[v].is_audio_message = true;

    b.label(c, "media:audio", "an audio file");
    b.attach(
        c,
        rowan,
        "Ringtone.aac",
        "audio/aac",
        "public.aac-audio",
        Some(AAC_AUDIO_SAMPLE.to_vec()),
        true,
        false,
    );

    b.label(c, "media:sticker", "a sticker");
    b.attach(
        c,
        rowan,
        "sticker.png",
        "image/png",
        "public.png",
        Some(png(6)),
        true,
        true,
    );

    b.label(c, "media:other", "a PDF, offered for download");
    b.attach(
        c,
        me,
        "Itinerary.pdf",
        "application/pdf",
        "com.adobe.pdf",
        Some(PDF.to_vec()),
        true,
        false,
    );

    b.label(
        c,
        "file:missing",
        "a photo the backup did not hold: placeholder",
    );
    b.attach(
        c,
        rowan,
        "IMG_2401.HEIC",
        "image/heic",
        "public.heic",
        None,
        true,
        false,
    );

    b.label(
        c,
        "file:no_path",
        "an attachment with no file path: placeholder",
    );
    b.attach(
        c,
        me,
        "IMG_2402.JPG",
        "image/jpeg",
        "public.jpeg",
        None,
        false,
        false,
    );

    b.label(
        c,
        "file:derivative_failed",
        "a damaged photo whose preview failed",
    );
    b.attach(
        c,
        rowan,
        "IMG_2403.HEIC",
        "image/heic",
        "public.heic",
        Some(crate::media::truncated(HEIC_SAMPLE, 200)),
        true,
        false,
    );

    b.label(c, "file:hidden", "text with an attachment iOS hides");
    let i = b.push(
        Some(c),
        rowan,
        Some("Text with a hidden attachment\u{FFFC}".into()),
    );
    let a = b.add_file(
        i,
        "hidden.png",
        "image/png",
        "public.png",
        Some(png(7)),
        true,
        false,
    );
    b.s.attachments[(a - 1) as usize].hidden = true;

    b.label(c, "balloon:url", "a link preview from stored data");
    b.say(c, rowan, TRAIL_LINK.url);
    let last = b.s.messages.len() - 1;
    b.s.messages[last].link_preview = Some(TRAIL_LINK);

    b.label(c, "balloon:app", "an iMessage app balloon");
    let i = b.push(Some(c), rowan, Some("\u{FFFC}".into()));
    b.s.messages[i].balloon_bundle_id = Some(APP_BALLOON);

    b.label(c, "poll_vote", "a vote in a poll");
    let i = b.push(Some(c), rowan, Some("Voted for “Saturday”".into()));
    b.s.messages[i].associated_message_guid = Some(format!("p:0/{first}"));
    b.s.messages[i].associated_message_type = 4000;

    b.label(c, "edited", "edited twice; earlier versions kept");
    let i = b.push(Some(c), me, Some("Meet at 9:30 at the east gate".into()));
    let t = b.s.messages[i].date;
    b.s.messages[i].date_edited = t + 240;
    b.s.messages[i].text_only_in_body = true;
    b.s.messages[i].edit_history = vec![
        (t, "Meet at 9 at the gate".into()),
        (t + 60, "Meet at 9:30 at the gate".into()),
        (t + 240, "Meet at 9:30 at the east gate".into()),
    ];

    b.label(c, "unsent", "a message I unsent");
    let i = b.push(Some(c), me, None);
    b.s.messages[i].date_retracted = b.s.messages[i].date + 20;
    b.s.messages[i].date_edited = b.s.messages[i].date + 20;

    b.label(c, "recovered", "deleted on the phone, recovered");
    let i = b.push(
        None,
        rowan,
        Some("This one was deleted, then recovered".into()),
    );
    b.s.messages[i].recoverable_in = Some(c);

    b.label(c, "reply", "a reply to the first message");
    b.reply(c, rowan, &first, "Replying to my own first message");

    b.label(
        c,
        "reply",
        "a reply, then a reaction inside its thread (counted under replies.on_reactions)",
    );
    let in_thread = b.reply(c, me, &first, "Another reply in the same thread");
    b.react(c, rowan, &in_thread, Tapback::Liked, false);
    let i = b.s.messages.len() - 1;
    b.s.messages[i].thread_originator_guid = Some(first.clone());
    b.s.messages[i].thread_originator_part = Some("0:0:10".to_owned());

    b.label(
        c,
        "problem:reply_target_missing",
        "a reply whose original is gone",
    );
    b.reply(
        c,
        rowan,
        &guid(0xDE1E_7ED0, 2),
        "Replying to a message that is not in the archive",
    );

    b.label(c, "effect:bubble", "sent with invisible ink");
    let i = b.push(Some(c), me, Some("Surprise: the cabin is booked".into()));
    b.s.messages[i].expressive_send_style_id =
        Some("com.apple.MobileSMS.expressivesend.invisibleink");

    b.label(c, "effect:screen", "sent with confetti");
    let i = b.push(Some(c), rowan, Some("Happy birthday!".into()));
    b.s.messages[i].expressive_send_style_id = Some("com.apple.messages.effect.CKConfettiEffect");

    b.label(c, "service:sms", "sent as a text message (green)");
    let i = b.push(Some(c), me, Some("No signal for iMessage up here".into()));
    b.s.messages[i].service = SERVICE_SMS;

    b.label(c, "service:rcs", "received over RCS");
    let i = b.push(Some(c), rowan, Some("Got it over RCS".into()));
    b.s.messages[i].service = "RCS";

    for (feature, kind, what) in [
        ("reaction:loved", Tapback::Loved, "a heart"),
        ("reaction:liked", Tapback::Liked, "a thumbs up"),
        ("reaction:disliked", Tapback::Disliked, "a thumbs down"),
        ("reaction:laughed", Tapback::Laughed, "ha ha"),
        ("reaction:emphasized", Tapback::Emphasized, "!!"),
        ("reaction:questioned", Tapback::Questioned, "?"),
        ("reaction:emoji", Tapback::Emoji, "an emoji reaction"),
    ] {
        b.label(c, feature, what);
        let target = b.say(c, rowan, &format!("React to this with {what}"));
        b.react(c, me, &target, kind, false);
    }

    b.label(c, "reaction:sticker", "a sticker placed on the message");
    let target = b.say(c, rowan, "Put a sticker on this");
    b.react(c, me, &target, Tapback::Loved, false);
    let i = b.s.messages.len() - 1;
    b.s.messages[i].associated_message_type = 1000;
    b.s.messages[i].text = Some("\u{FFFC}".into());
    b.add_file(
        i,
        "sticker-reaction.png",
        "image/png",
        "public.png",
        Some(png(8)),
        true,
        true,
    );

    b.label(c, "reaction:removed", "a heart added, then taken back");
    let target = b.say(c, rowan, "I'll take my heart back");
    b.react(c, me, &target, Tapback::Loved, false);
    b.react(c, me, &target, Tapback::Loved, true);

    b.label(
        c,
        "text",
        "a removal with no reaction before it, and one of another kind (counted, not shown)",
    );
    let target = b.say(c, rowan, "Removals that match nothing");
    b.react(c, me, &target, Tapback::Laughed, true);
    b.react(c, rowan, &target, Tapback::Liked, false);
    b.react(c, rowan, &target, Tapback::Loved, true);
    // Delivered into another chat row (SMS) for the same person: that chat
    // holds only this reaction and is not listed.
    b.react(
        REACTIONS_ONLY_CHAT,
        rowan,
        &target,
        Tapback::Emphasized,
        false,
    );

    b.label(c, "reaction:several", "reactions from both of us");
    let target = b.say(c, me, "Two reactions on this one");
    b.react(c, rowan, &target, Tapback::Laughed, false);
    b.react(c, me, &target, Tapback::Emphasized, false);

    b.label(c, "event:location_sharing", "location sharing");
    b.system(c, rowan, 4, 0, 0, None);
    b.label(c, "event:audio_kept", "a voice message kept");
    b.system(c, rowan, 5, 0, 0, None);
    b.label(c, "event:shareplay", "a FaceTime / SharePlay event");
    b.system(c, me, 6, 0, 0, None);

    b.label(
        c,
        "problem:unknown_item_type",
        "an item type this version does not know",
    );
    b.system(c, rowan, 99, 0, 0, None);

    b.label(
        c,
        "problem:unknown_associated_type",
        "an associated type this version does not know",
    );
    let i = b.push(Some(c), rowan, Some("Associated type 5000".into()));
    b.s.messages[i].associated_message_guid = Some(format!("p:0/{first}"));
    b.s.messages[i].associated_message_type = 5000;

    b.label(
        c,
        "file:no_record",
        "an attachment placeholder with no attachment record",
    );
    b.push(Some(c), rowan, Some("\u{FFFC}".into()));

    b.label(
        c,
        "audio_message",
        "a voice message whose file has no record (expired)",
    );
    let i = b.push(Some(c), rowan, Some("\u{FFFC}".into()));
    b.s.messages[i].is_audio_message = true;

    b.label(
        c,
        "audio_message",
        "a voice message stored as M4A (media:audio)",
    );
    let v = b.attach(
        c,
        me,
        "Audio Message.m4a",
        "audio/x-m4a",
        "com.apple.m4a-audio",
        Some(AAC_AUDIO_SAMPLE.to_vec()),
        true,
        false,
    );
    b.s.messages[v].is_audio_message = true;

    b.label(
        c,
        "file:missing",
        "two missing photos in one message (two files, one item)",
    );
    let i = b.push(Some(c), rowan, Some("\u{FFFC}\u{FFFC}".into()));
    b.add_file(
        i,
        "IMG_2501.HEIC",
        "image/heic",
        "public.heic",
        None,
        true,
        false,
    );
    b.add_file(
        i,
        "IMG_2502.HEIC",
        "image/heic",
        "public.heic",
        None,
        true,
        false,
    );

    b.label(
        c,
        "problem:empty:no_body",
        "a message with no text and no body",
    );
    b.push(Some(c), rowan, None);

    b.label(
        c,
        "problem:empty:body_undecoded",
        "a body that does not decode",
    );
    let i = b.push(Some(c), rowan, None);
    b.s.messages[i].attributed_body_override =
        Some(b"streamtyped\x81\xe8\x03\x84\x01@\x84\x84\x84".to_vec());

    b.label(c, "problem:empty:blank_text", "text of spaces only");
    b.say(c, rowan, "   ");

    b.label(
        c,
        "problem:empty:only_hidden_attachments",
        "only a hidden attachment",
    );
    let i = b.push(Some(c), rowan, Some("\u{FFFC}".into()));
    let a = b.add_file(
        i,
        "hidden-only.png",
        "image/png",
        "public.png",
        Some(png(9)),
        true,
        false,
    );
    b.s.attachments[(a - 1) as usize].hidden = true;

    b.wait(5);
    b.say(
        c,
        me,
        "That is every kind. The group version shows group events.",
    );
}

fn kinds_group(b: &mut Builder) {
    let (c, me, rowan, avery, bram, cleo) = (
        KINDS_GROUP,
        From::Me,
        From::H(ROWAN),
        From::H(1),
        From::H(4),
        From::H(5),
    );
    b.at(2025, 4, 21, 18, 0);
    b.say(
        c,
        me,
        "The group version: group events and several senders.",
    );

    b.label(c, "text", "messages from three people in a row");
    let q = b.say(c, rowan, "Rowan here");
    b.say(c, avery, "Avery here");
    b.say(c, bram, "Bram here");

    b.label(c, "reaction:several", "reactions from three people");
    let t = b.say(c, rowan, "React if you're coming Saturday");
    b.react(c, avery, &t, Tapback::Loved, false);
    b.react(c, bram, &t, Tapback::Liked, false);
    b.react(c, me, &t, Tapback::Emoji, false);

    b.label(c, "reply", "a reply in a group");
    b.reply(c, bram, &q, "Hi Rowan");

    b.label(c, "event:renamed", "group renamed");
    b.system(c, avery, 2, 0, 0, Some(KINDS_GROUP_TITLE));
    b.label(c, "event:added", "Bram added Cleo");
    b.system(c, bram, 1, 0, 5, None);
    b.say(c, cleo, "Thanks for adding me");
    b.label(c, "event:removed", "Rowan removed Cleo");
    b.system(c, rowan, 1, 1, 5, None);
    b.label(c, "event:icon_changed", "group photo changed");
    b.system(c, avery, 3, 1, 0, None);
    b.label(c, "event:icon_removed", "group photo removed");
    b.system(c, avery, 3, 2, 0, None);
    b.label(c, "event:background_changed", "chat background changed");
    b.system(c, me, 3, 4, 0, None);
    b.label(c, "event:background_removed", "chat background removed");
    b.system(c, me, 3, 6, 0, None);
    b.label(c, "event:number_changed", "Rowan changed their number");
    b.system(c, rowan, 1, 0, ROWAN, None);
    b.label(c, "event:other", "a group event this version does not name");
    b.system(c, avery, 3, 3, 0, None);
    b.label(c, "event:left", "Bram left");
    b.system(c, bram, 3, 0, 0, None);
    b.wait(5);
    b.say(c, me, "That is every group event.");
}

/// A long 1:1 conversation for scrolling: runs of quick messages, hour and
/// day gaps, and a reply or reaction now and then.
fn large_chat(b: &mut Builder) {
    let theo = THEO;
    let mut rowid = b.s.messages.iter().map(|m| m.rowid).max().unwrap_or(0);
    let mut now = utc(2019, 3, 1, 8, 0);
    let end = utc(2025, 4, 1, 0, 0);
    // Spread the messages over six years.
    let step = (end - now) / LARGE_CHAT_MESSAGES as i64;
    let mut targets: Vec<String> = Vec::new();
    for n in 0..LARGE_CHAT_MESSAGES {
        rowid += 1;
        let from_me = (n * 7 + n / 5) % 3 == 0;
        let mut m = Message {
            rowid,
            guid: guid(0x1A26_0000, n),
            text: Some(format!(
                "{} (#{})",
                FILLER[(n * 11 + n / 13) % FILLER.len()],
                n + 1
            )),
            text_only_in_body: n % 9 == 4,
            handle_id: theo,
            service: SERVICE_IMESSAGE,
            date: now,
            date_delivered: if from_me { now + 2 } else { 0 },
            date_read: now + 30,
            is_from_me: from_me,
            is_read: true,
            is_delivered: true,
            chat: Some(LARGE_CHAT),
            ..Message::default()
        };
        if n % 97 == 50 && !targets.is_empty() {
            let t = &targets[targets.len() - 1];
            m.thread_originator_guid = Some(t.clone());
            m.thread_originator_part = Some("0:0:10".to_owned());
        }
        targets.push(m.guid.clone());
        b.s.messages.push(m);
        if n % 61 == 30 {
            rowid += 1;
            let target = targets[targets.len() - 1].clone();
            b.s.messages.push(Message {
                rowid,
                guid: guid(0x1A27_0000, n),
                text: Some("Loved a message".to_owned()),
                handle_id: theo,
                service: SERVICE_IMESSAGE,
                date: now + 20,
                date_read: now + 30,
                is_from_me: !from_me,
                is_read: true,
                is_delivered: true,
                chat: Some(LARGE_CHAT),
                associated_message_guid: Some(format!("p:0/{target}")),
                associated_message_type: Tapback::Loved as i64,
                associated_message_range_length: 10,
                ..Message::default()
            });
        }
        // Mostly seconds apart, with hour, day and week gaps.
        now += match n % 40 {
            0 => step * 12,
            7 | 23 => step * 4,
            _ => step / 3,
        }
        .max(5);
        now = now.min(end);
    }
}
