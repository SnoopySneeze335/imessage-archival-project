//! The render model: how every archived message turns into what the UI
//! draws (decision 0010).
//!
//! The browser never interprets archive rows. This module classifies each
//! message into one [`Body`] kind, attaches its reactions, attachments and
//! flags, and names every case it recognises with a feature label (see
//! [`FEATURES`]). `imessage-ap check-render` runs every message in the archive
//! through the same code and reports counts per label, and problems with
//! their GUIDs, never text.
//!
//! Adding a kind or state means: a label in [`FEATURES`], an example in the
//! demo fixture's "All message kinds" chats, and the UI to draw it. Labels the
//! view does not produce yet wait in [`PENDING`].

pub mod check;

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use rusqlite::{Connection, params_from_iter};
use serde::Serialize;

use crate::media::MediaKind;

/// Every label the view can give an item, with what it means. The order is
/// the order of the demo's "All message kinds" chats.
pub const FEATURES: &[(&str, &str)] = &[
    // Body kinds: exactly one per item.
    (
        "message",
        "An ordinary message: text, attachments or an app balloon",
    ),
    (
        "unsent",
        "A message its sender unsent: a labelled placeholder",
    ),
    ("event", "A system event: a centred grey line"),
    (
        "unknown",
        "An item type this version cannot draw: an Unsupported bubble",
    ),
    // Content of a message.
    ("text", "Text"),
    ("subject", "A subject line above the text"),
    ("emoji_only", "Text of 1-3 emoji only, drawn large"),
    ("attachment", "At least one attachment"),
    ("attachments_multiple", "Several attachments in one message"),
    ("audio_message", "A voice message recorded in Messages"),
    ("balloon:url", "A link preview, drawn from stored data only"),
    (
        "balloon:app",
        "An iMessage app balloon (game, payment, ...)",
    ),
    ("poll_vote", "A vote in a poll"),
    // Message flags.
    ("edited", "Edited after sending"),
    (
        "recovered",
        "Deleted on the phone, recovered from Recently Deleted",
    ),
    ("reply", "A reply to an earlier message"),
    (
        "effect:bubble",
        "Sent with a bubble effect (slam, loud, gentle, invisible ink)",
    ),
    (
        "effect:screen",
        "Sent with a screen effect (confetti, balloons, ...)",
    ),
    ("service:sms", "Sent or received as SMS (green when sent)"),
    ("service:rcs", "Sent or received as RCS"),
    // Attachments, by what the browser does with them.
    (
        "media:web_image",
        "An image browsers show as it is (JPEG, PNG, GIF, WebP)",
    ),
    (
        "media:image",
        "An image shown through its preview (HEIC, TIFF, ...)",
    ),
    ("media:video", "A video"),
    (
        "media:voice",
        "A voice recording (CAF, AMR), played through its conversion",
    ),
    ("media:audio", "An audio file browsers play as it is"),
    ("media:sticker", "A sticker"),
    ("media:other", "Any other file, offered for download"),
    // Attachment file states.
    ("file:ok", "The file and everything made from it are ready"),
    (
        "file:missing",
        "The backup did not hold the file: placeholder with name and type",
    ),
    (
        "file:no_path",
        "The record has no file path: placeholder with name and type",
    ),
    (
        "file:derivative_failed",
        "The file is here but a thumbnail, preview or conversion failed",
    ),
    (
        "file:not_copied",
        "Not copied from the backup yet (run `attachments`)",
    ),
    (
        "file:derivative_pending",
        "A preview or conversion is still queued (run `derive`)",
    ),
    ("file:hidden", "An attachment iOS marks hidden"),
    // Reactions on an item.
    ("reaction:loved", "Tapback: heart"),
    ("reaction:liked", "Tapback: thumbs up"),
    ("reaction:disliked", "Tapback: thumbs down"),
    ("reaction:laughed", "Tapback: ha ha"),
    ("reaction:emphasized", "Tapback: !!"),
    ("reaction:questioned", "Tapback: ?"),
    ("reaction:emoji", "An emoji reaction"),
    ("reaction:sticker", "A sticker placed on the message"),
    (
        "reaction:removed",
        "A reaction later taken back (not shown)",
    ),
    ("reaction:several", "Reactions from more than one person"),
    // System events.
    ("event:renamed", "Group renamed"),
    ("event:added", "Someone added to the group"),
    ("event:removed", "Someone removed from the group"),
    ("event:left", "Someone left the group"),
    ("event:icon_changed", "Group photo changed"),
    ("event:icon_removed", "Group photo removed"),
    ("event:background_changed", "Chat background changed"),
    ("event:background_removed", "Chat background removed"),
    ("event:number_changed", "A participant changed their number"),
    (
        "event:location_sharing",
        "Location sharing started or stopped",
    ),
    ("event:audio_kept", "A voice message was kept"),
    ("event:shareplay", "A SharePlay or FaceTime event"),
    (
        "event:other",
        "A group event of a type this version does not name",
    ),
    // Problems: counted with GUIDs by check-render.
    (
        "problem:unknown_item_type",
        "Item type this version does not know",
    ),
    (
        "problem:unknown_associated_type",
        "Associated-message type this version does not know",
    ),
    ("problem:empty", "A message with nothing to show"),
    (
        "problem:reply_target_missing",
        "A reply whose original is not in the archive",
    ),
    (
        "problem:orphan",
        "A message in no conversation (listed under Unfiled)",
    ),
    (
        "problem:reaction_target_missing",
        "A reaction whose message is not in the archive (hidden)",
    ),
];

/// Labels not produced yet: the milestone that adds each. That PR adds its
/// [`FEATURES`] entry and demo example and removes it from here.
pub const PENDING: &[(&str, &str)] = &[
    (
        "link_preview:content",
        "4b: title, summary and site parsed from payload_data",
    ),
    (
        "edit_history",
        "4b: earlier versions of an edited message, from message_edit",
    ),
    ("reply:quote", "4b: the quoted original above a reply"),
    (
        "parts",
        "4b: text and attachments of one message in their original order",
    ),
    (
        "attachment:url",
        "4c: thumbnail, preview and original URLs for each attachment",
    ),
];

/// Labels that exist in the demo data but cannot sit in a conversation: the
/// coverage test looks for them anywhere.
pub const OUTSIDE_CHATS: &[&str] = &["problem:orphan", "problem:reaction_target_missing"];

/// States that only exist while `attachments` or `derive` has work left. The
/// demo runs both to the end, so it has no example of them.
pub const TRANSIENT: &[&str] = &["file:not_copied", "file:derivative_pending"];

/// What an item is. Exactly one per timeline item.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Message {
        /// Text with attachment placeholders (U+FFFC) removed; `None` when
        /// nothing is left.
        text: Option<String>,
        subject: Option<String>,
        attachments: Vec<AttachmentView>,
        /// `url` or `app`.
        balloon: Option<&'static str>,
        /// The app's bundle ID, for app balloons.
        app: Option<String>,
        audio_message: bool,
    },
    Unsent,
    Event {
        /// The label without its `event:` prefix, such as `renamed`.
        event: &'static str,
        /// The other person in the event (added, removed, number changed).
        other: Option<HandleView>,
        /// The new name, for `renamed`.
        title: Option<String>,
    },
    Unknown {
        item_type: i64,
    },
}

/// A raw address. Names come in Milestone 4d.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HandleView {
    pub id: i64,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AttachmentView {
    pub guid: String,
    /// The file's original name.
    pub name: Option<String>,
    pub mime_type: Option<String>,
    /// `media:` label without its prefix.
    pub media: &'static str,
    /// `file:` label without its prefix.
    pub state: &'static str,
    pub sha256: Option<String>,
    pub bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReactionView {
    /// `reaction:` label without its prefix.
    pub kind: &'static str,
    pub emoji: Option<String>,
    pub from_me: bool,
    pub sender: Option<HandleView>,
    /// Which part of the message it is on.
    pub part: i64,
}

/// One timeline item.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Item {
    pub id: i64,
    pub guid: String,
    /// Milliseconds since 1970, UTC.
    pub sent_at: Option<i64>,
    pub from_me: bool,
    pub sender: Option<HandleView>,
    /// `iMessage`, `SMS`, `RCS`, ...
    pub service: Option<String>,
    pub body: Body,
    pub edited: bool,
    pub recovered: bool,
    /// GUID of the message replied to.
    pub reply_to: Option<String>,
    /// The expressive send style's bundle ID.
    pub effect: Option<String>,
    pub reactions: Vec<ReactionView>,
    /// Every label from [`FEATURES`] this item shows.
    pub features: Vec<&'static str>,
}

/// Every conversation's items in order, as message ids, built in one pass.
/// Reactions are not items: they sit on their targets.
#[derive(Debug, Default)]
pub struct Timeline {
    /// Chat id → message ids, oldest first.
    pub chats: BTreeMap<i64, Vec<i64>>,
    /// Messages in no conversation, oldest first.
    pub unfiled: Vec<i64>,
}

impl Timeline {
    pub fn build(db: &Connection) -> Result<Self> {
        let mut t = Self::default();
        let mut stmt = db.prepare(
            "SELECT mc.chat_id, m.id FROM message m
             JOIN message_chat mc ON mc.message_id = m.id
             WHERE NOT EXISTS (SELECT 1 FROM reaction r WHERE r.message_id = m.id)
             ORDER BY mc.chat_id, m.sent_at, m.id",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            t.chats.entry(r.get(0)?).or_default().push(r.get(1)?);
        }
        let mut stmt = db.prepare(
            "SELECT m.id FROM message m
             WHERE NOT EXISTS (SELECT 1 FROM message_chat mc WHERE mc.message_id = m.id)
               AND NOT EXISTS (SELECT 1 FROM reaction r WHERE r.message_id = m.id)
             ORDER BY m.sent_at, m.id",
        )?;
        t.unfiled = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(t)
    }
}

/// Raw columns of one message.
struct Row {
    id: i64,
    guid: String,
    sent_at: Option<i64>,
    from_me: bool,
    sender: Option<HandleView>,
    service: Option<String>,
    text: Option<String>,
    subject: Option<String>,
    item_type: i64,
    group_action_type: i64,
    other: Option<HandleView>,
    group_title: Option<String>,
    associated_type: Option<i64>,
    reply_to: Option<String>,
    balloon: Option<String>,
    effect: Option<String>,
    audio: bool,
    unsent: bool,
    recovered: bool,
    edited: bool,
}

const CHUNK: usize = 500;

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// Loads the items for `ids`, in the order given.
pub fn load(db: &Connection, ids: &[i64]) -> Result<Vec<Item>> {
    let mut out = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(CHUNK) {
        out.extend(load_chunk(db, chunk)?);
    }
    Ok(out)
}

fn handle_view(id: Option<i64>, address: Option<String>) -> Option<HandleView> {
    match (id, address) {
        (Some(id), Some(address)) => Some(HandleView { id, address }),
        _ => None,
    }
}

fn load_chunk(db: &Connection, ids: &[i64]) -> Result<Vec<Item>> {
    let ph = placeholders(ids.len());
    let mut rows: HashMap<i64, Row> = HashMap::with_capacity(ids.len());
    {
        let mut stmt = db.prepare_cached(&format!(
            "SELECT m.id, m.guid, m.sent_at, m.is_from_me, m.sender_handle_id, hs.address, m.service,
                    m.text, m.subject, m.item_type, m.group_action_type, m.other_handle_id, ho.address,
                    m.group_title, m.associated_message_type, m.reply_to_guid, m.balloon_bundle_id,
                    m.expressive_send_style_id, m.is_audio_message, m.is_unsent, m.is_recovered_deleted,
                    m.edited_at
             FROM message m
             LEFT JOIN handle hs ON hs.id = m.sender_handle_id
             LEFT JOIN handle ho ON ho.id = m.other_handle_id
             WHERE m.id IN ({ph})"
        ))?;
        let mut q = stmt.query(params_from_iter(ids))?;
        while let Some(r) = q.next()? {
            let row = Row {
                id: r.get(0)?,
                guid: r.get(1)?,
                sent_at: r.get::<_, Option<i64>>(2)?.map(|ns| ns / 1_000_000),
                from_me: r.get(3)?,
                sender: handle_view(r.get(4)?, r.get(5)?),
                service: r.get(6)?,
                text: r.get(7)?,
                subject: r.get(8)?,
                item_type: r.get(9)?,
                group_action_type: r.get(10)?,
                other: handle_view(r.get(11)?, r.get(12)?),
                group_title: r.get(13)?,
                associated_type: r.get(14)?,
                reply_to: r.get(15)?,
                balloon: r.get(16)?,
                effect: r.get(17)?,
                audio: r.get(18)?,
                unsent: r.get(19)?,
                recovered: r.get(20)?,
                edited: r.get::<_, Option<i64>>(21)?.is_some(),
            };
            rows.insert(row.id, row);
        }
    }

    let attachments = load_attachments(db, ids)?;
    let guids: Vec<&str> = rows.values().map(|r| r.guid.as_str()).collect();
    let reactions = load_reactions(db, &guids)?;
    let reply_targets: Vec<&str> = rows
        .values()
        .filter_map(|r| r.reply_to.as_deref())
        .collect();
    let existing = existing_guids(db, &reply_targets)?;

    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(row) = rows.remove(id) else { continue };
        let atts = attachments.get(id).cloned().unwrap_or_default();
        let reacts = reactions.get(&row.guid).cloned().unwrap_or_default();
        out.push(build(row, atts, reacts, &existing));
    }
    Ok(out)
}

/// The label of an attachment's file state, from its record, its blob and
/// the derivatives made from it.
fn file_state(
    missing: Option<&str>,
    sha256: Option<&str>,
    derivative_states: &[String],
) -> &'static str {
    match missing {
        Some("not_in_backup") => return "missing",
        Some(_) => return "no_path",
        None => {}
    }
    if sha256.is_none() {
        return "not_copied";
    }
    if derivative_states.iter().any(|s| s == "failed") {
        "derivative_failed"
    } else if derivative_states.iter().any(|s| s == "pending") {
        "derivative_pending"
    } else {
        "ok"
    }
}

/// Attachments by message id, in attachment id order, each with whether
/// iOS hides it.
fn load_attachments(
    db: &Connection,
    ids: &[i64],
) -> Result<HashMap<i64, Vec<(AttachmentView, bool)>>> {
    let mut stmt = db.prepare_cached(&format!(
        "SELECT ma.message_id, a.guid, a.transfer_name, a.mime_type, a.uti, a.is_sticker,
                a.hide_attachment, a.sha256, a.missing, a.total_bytes, b.media_kind,
                (SELECT group_concat(d.status) FROM derivative d WHERE d.sha256 = a.sha256)
         FROM message_attachment ma
         JOIN attachment a ON a.id = ma.attachment_id
         LEFT JOIN blob b ON b.sha256 = a.sha256
         WHERE ma.message_id IN ({})
         ORDER BY ma.message_id, a.id",
        placeholders(ids.len())
    ))?;
    let mut out: HashMap<i64, Vec<(AttachmentView, bool)>> = HashMap::new();
    let mut q = stmt.query(params_from_iter(ids))?;
    while let Some(r) = q.next()? {
        let name: Option<String> = r.get(2)?;
        let mime: Option<String> = r.get(3)?;
        let uti: Option<String> = r.get(4)?;
        let sticker: bool = r.get(5)?;
        let hidden: bool = r.get(6)?;
        let sha256: Option<String> = r.get(7)?;
        let missing: Option<String> = r.get(8)?;
        let blob_kind: Option<String> = r.get(10)?;
        let derivs: Vec<String> = r
            .get::<_, Option<String>>(11)?
            .map(|s| s.split(',').map(str::to_owned).collect())
            .unwrap_or_default();
        let media = if sticker {
            "sticker"
        } else {
            blob_kind
                .as_deref()
                .map(MediaKind::parse)
                .unwrap_or_else(|| {
                    MediaKind::classify(mime.as_deref(), uti.as_deref(), name.as_deref())
                })
                .as_str()
        };
        let view = AttachmentView {
            guid: r.get(1)?,
            name,
            mime_type: mime,
            media,
            state: file_state(missing.as_deref(), sha256.as_deref(), &derivs),
            sha256,
            bytes: r.get(9)?,
        };
        out.entry(r.get(0)?).or_default().push((view, hidden));
    }
    Ok(out)
}

/// A reaction message, as stored.
#[derive(Debug, Clone)]
struct RawReaction {
    part: i64,
    kind: i64,
    emoji: Option<String>,
    removal: bool,
    from_me: bool,
    sender: Option<HandleView>,
}

/// The classic tapback, emoji or sticker an associated-message type names,
/// added or removed.
pub fn reaction_kind(kind: i64) -> Option<&'static str> {
    Some(match kind {
        1000 | 2007 | 3007 => "sticker",
        2000 | 3000 => "loved",
        2001 | 3001 => "liked",
        2002 | 3002 => "disliked",
        2003 | 3003 => "laughed",
        2004 | 3004 => "emphasized",
        2005 | 3005 => "questioned",
        2006 | 3006 => "emoji",
        _ => return None,
    })
}

/// Reactions by target GUID, in the order they were sent.
fn load_reactions(db: &Connection, guids: &[&str]) -> Result<HashMap<String, Vec<RawReaction>>> {
    let mut out: HashMap<String, Vec<RawReaction>> = HashMap::new();
    if guids.is_empty() {
        return Ok(out);
    }
    let mut stmt = db.prepare_cached(&format!(
        "SELECT r.target_guid, r.target_part, r.kind, r.emoji, r.is_removal, m.is_from_me,
                m.sender_handle_id, h.address
         FROM reaction r
         JOIN message m ON m.id = r.message_id
         LEFT JOIN handle h ON h.id = m.sender_handle_id
         WHERE r.target_guid IN ({})
         ORDER BY m.sent_at, m.id",
        placeholders(guids.len())
    ))?;
    let mut q = stmt.query(params_from_iter(guids))?;
    while let Some(r) = q.next()? {
        out.entry(r.get(0)?).or_default().push(RawReaction {
            part: r.get(1)?,
            kind: r.get(2)?,
            emoji: r.get(3)?,
            removal: r.get(4)?,
            from_me: r.get(5)?,
            sender: handle_view(r.get(6)?, r.get(7)?),
        });
    }
    Ok(out)
}

fn existing_guids(db: &Connection, guids: &[&str]) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    for chunk in guids.chunks(CHUNK) {
        let mut stmt = db.prepare_cached(&format!(
            "SELECT guid FROM message WHERE guid IN ({})",
            placeholders(chunk.len())
        ))?;
        let mut q = stmt.query(params_from_iter(chunk))?;
        while let Some(r) = q.next()? {
            out.insert(r.get(0)?);
        }
    }
    Ok(out)
}

/// The reactions still standing after removals, one per person and part:
/// a newer tapback from the same person replaces the older one, and a
/// removal takes back a matching one. Returns them and whether any removal
/// took effect.
fn standing_reactions(raw: Vec<RawReaction>) -> (Vec<ReactionView>, bool) {
    // Keyed by who reacted (from me, sender handle) and on which part.
    type Who = (bool, Option<i64>, i64);
    let mut current: Vec<(Who, ReactionView)> = Vec::new();
    let mut removed = false;
    for r in raw {
        let Some(kind) = reaction_kind(r.kind) else {
            continue;
        };
        let who = (r.from_me, r.sender.as_ref().map(|h| h.id), r.part);
        let pos = current.iter().position(|(k, _)| *k == who);
        if r.removal {
            if let Some(p) = pos
                && current[p].1.kind == kind
            {
                current.remove(p);
                removed = true;
            }
            continue;
        }
        let view = ReactionView {
            kind,
            emoji: r.emoji,
            from_me: r.from_me,
            sender: r.sender,
            part: r.part,
        };
        match pos {
            // Stickers stack; tapbacks replace.
            Some(p) if kind != "sticker" && current[p].1.kind != "sticker" => current[p].1 = view,
            _ => current.push((who, view)),
        }
    }
    (current.into_iter().map(|(_, v)| v).collect(), removed)
}

/// The group event an item type and action name, as in imessage-database's
/// `GroupAction` and `Announcement`.
fn event_kind(row: &Row) -> Option<&'static str> {
    Some(match (row.item_type, row.group_action_type) {
        (1, 0)
            if row.other.is_some()
                && row.other.as_ref().map(|h| h.id) == row.sender.as_ref().map(|h| h.id) =>
        {
            "number_changed"
        }
        (1, 0) => "added",
        (1, 1) => "removed",
        (2, _) => "renamed",
        (3, 0) => "left",
        (3, 1) => "icon_changed",
        (3, 2) => "icon_removed",
        (3, 4) => "background_changed",
        (3, 6) => "background_removed",
        (4, _) => "location_sharing",
        (5, _) => "audio_kept",
        (6, _) => "shareplay",
        (1 | 3, _) => "other",
        _ => return None,
    })
}

const URL_BALLOON: &str = "com.apple.messages.URLBalloonProvider";
const APP_EXTENSION: &str = "com.apple.messages.MSMessageExtensionBalloonPlugin";

/// `true` when `text` is one to three emoji and nothing else.
fn emoji_only(text: &str) -> bool {
    let mut count = 0;
    for c in text.chars() {
        if c.is_whitespace() || matches!(c, '\u{200D}' | '\u{FE0F}' | '\u{20E3}') {
            continue;
        }
        let emoji = matches!(c as u32,
            0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF);
        if !emoji {
            return false;
        }
        // Skin tones modify the emoji before them.
        if !matches!(c as u32, 0x1F3FB..=0x1F3FF) {
            count += 1;
        }
    }
    (1..=3).contains(&count)
}

fn build(
    row: Row,
    attachments: Vec<(AttachmentView, bool)>,
    reactions: Vec<RawReaction>,
    existing: &HashSet<String>,
) -> Item {
    let mut f: Vec<&'static str> = Vec::new();
    let body = if row.item_type == 0 {
        if row.unsent {
            f.push("unsent");
            Body::Unsent
        } else {
            f.push("message");
            let text = row
                .text
                .as_deref()
                .map(|t| t.replace('\u{FFFC}', ""))
                .filter(|t| !t.trim().is_empty());
            let subject = row.subject.clone().filter(|s| !s.trim().is_empty());
            if let Some(t) = &text {
                f.push("text");
                if emoji_only(t) {
                    f.push("emoji_only");
                }
            }
            if subject.is_some() {
                f.push("subject");
            }
            let bundle = row.balloon.as_deref();
            let balloon = match bundle {
                Some(URL_BALLOON) => Some("url"),
                Some(_) => Some("app"),
                None => None,
            };
            if let Some(b) = balloon {
                f.push(if b == "url" {
                    "balloon:url"
                } else {
                    "balloon:app"
                });
            }
            let app = (balloon == Some("app")).then(|| {
                let b = bundle.unwrap_or_default();
                b.strip_prefix(APP_EXTENSION)
                    .and_then(|rest| rest.rsplit(':').next())
                    .unwrap_or(b)
                    .to_owned()
            });
            let visible: Vec<AttachmentView> = attachments
                .iter()
                .filter(|(_, hidden)| !hidden)
                .map(|(a, _)| a.clone())
                .collect();
            if attachments.iter().any(|(_, hidden)| *hidden) {
                f.push("file:hidden");
            }
            if !visible.is_empty() {
                f.push("attachment");
            }
            if visible.len() > 1 {
                f.push("attachments_multiple");
            }
            for a in &visible {
                f.push(media_label(a.media));
                f.push(state_label(a.state));
            }
            if row.audio {
                f.push("audio_message");
            }
            let vote = row.associated_type == Some(4000);
            if vote {
                f.push("poll_vote");
            }
            if let Some(t) = row.associated_type
                && !matches!(t, 0 | 2 | 3 | 4000)
            {
                f.push("problem:unknown_associated_type");
            }
            if text.is_none()
                && subject.is_none()
                && visible.is_empty()
                && balloon.is_none()
                && !vote
            {
                f.push("problem:empty");
            }
            Body::Message {
                text,
                subject,
                attachments: visible,
                balloon,
                app,
                audio_message: row.audio,
            }
        }
    } else if let Some(event) = event_kind(&row) {
        f.push("event");
        f.push(event_label(event));
        Body::Event {
            event,
            other: row.other.clone(),
            title: (event == "renamed")
                .then(|| row.group_title.clone())
                .flatten(),
        }
    } else {
        f.push("unknown");
        f.push("problem:unknown_item_type");
        Body::Unknown {
            item_type: row.item_type,
        }
    };

    if row.edited && !row.unsent {
        f.push("edited");
    }
    if row.recovered {
        f.push("recovered");
    }
    let reply_to = row.reply_to.clone().filter(|g| *g != row.guid);
    if let Some(target) = &reply_to {
        f.push("reply");
        if !existing.contains(target) {
            f.push("problem:reply_target_missing");
        }
    }
    let effect = row.effect.clone().filter(|e| !e.is_empty());
    if let Some(e) = &effect {
        f.push(if e.starts_with("com.apple.MobileSMS.expressivesend.") {
            "effect:bubble"
        } else {
            "effect:screen"
        });
    }
    match row.service.as_deref() {
        Some("SMS") => f.push("service:sms"),
        Some("RCS") => f.push("service:rcs"),
        _ => {}
    }

    let (reactions, removed) = standing_reactions(reactions);
    for r in &reactions {
        f.push(reaction_label(r.kind));
    }
    if removed {
        f.push("reaction:removed");
    }
    let people: HashSet<(bool, Option<i64>)> = reactions
        .iter()
        .map(|r| (r.from_me, r.sender.as_ref().map(|h| h.id)))
        .collect();
    if people.len() > 1 {
        f.push("reaction:several");
    }

    // One label each, in FEATURES order.
    f.sort_by_key(|l| {
        FEATURES
            .iter()
            .position(|(n, _)| n == l)
            .unwrap_or(usize::MAX)
    });
    f.dedup();

    Item {
        id: row.id,
        guid: row.guid,
        sent_at: row.sent_at,
        from_me: row.from_me,
        sender: row.sender,
        service: row.service,
        body,
        edited: row.edited && !row.unsent,
        recovered: row.recovered,
        reply_to,
        effect,
        reactions,
        features: f,
    }
}

/// The `&'static` label for `prefix:name`, from [`FEATURES`].
fn label(prefix: &str, name: &str) -> &'static str {
    FEATURES
        .iter()
        .find(|(l, _)| l.strip_prefix(prefix).and_then(|r| r.strip_prefix(':')) == Some(name))
        .map_or("unknown", |(l, _)| l)
}

fn media_label(media: &str) -> &'static str {
    label("media", media)
}

fn state_label(state: &str) -> &'static str {
    label("file", state)
}

fn event_label(event: &str) -> &'static str {
    label("event", event)
}

fn reaction_label(kind: &str) -> &'static str {
    label("reaction", kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_is_listed_once() {
        let mut seen = HashSet::new();
        for (l, d) in FEATURES {
            assert!(seen.insert(*l), "{l} listed twice");
            assert!(!d.is_empty());
        }
        for (p, _) in PENDING {
            assert!(!seen.contains(p), "{p} is both built and pending");
        }
        for l in OUTSIDE_CHATS.iter().chain(TRANSIENT) {
            assert!(seen.contains(l), "{l}");
        }
        for m in [
            "web_image",
            "image",
            "video",
            "voice",
            "audio",
            "other",
            "sticker",
        ] {
            assert_ne!(media_label(m), "unknown", "{m}");
        }
        for s in [
            "ok",
            "missing",
            "no_path",
            "derivative_failed",
            "not_copied",
            "derivative_pending",
        ] {
            assert_ne!(state_label(s), "unknown", "{s}");
        }
        for k in 1000..4000 {
            if let Some(r) = reaction_kind(k) {
                assert_ne!(reaction_label(r), "unknown", "{k}");
            }
        }
    }

    #[test]
    fn emoji_only_text() {
        assert!(emoji_only("🎉"));
        assert!(emoji_only("👍🏽 🎉"));
        assert!(emoji_only("👨‍👩‍👧"));
        assert!(emoji_only("🇺🇸"));
        assert!(!emoji_only("🎉🎉🎉🎉"));
        assert!(!emoji_only("ok 🎉"));
        assert!(!emoji_only(""));
    }

    fn r(kind: i64, removal: bool, who: Option<i64>) -> RawReaction {
        RawReaction {
            part: 0,
            kind,
            emoji: None,
            removal,
            from_me: who.is_none(),
            sender: who.map(|id| HandleView {
                id,
                address: String::new(),
            }),
        }
    }

    #[test]
    fn reactions_replace_and_remove() {
        // Loved, then removed it, then liked: one like.
        let (v, removed) = standing_reactions(vec![
            r(2000, false, Some(1)),
            r(3000, true, Some(1)),
            r(2001, false, Some(1)),
        ]);
        assert_eq!(v.iter().map(|r| r.kind).collect::<Vec<_>>(), ["liked"]);
        assert!(removed);
        // A newer tapback replaces the older; another person adds their own.
        let (v, removed) = standing_reactions(vec![
            r(2000, false, Some(1)),
            r(2003, false, Some(1)),
            r(2004, false, None),
        ]);
        assert_eq!(
            v.iter().map(|r| r.kind).collect::<Vec<_>>(),
            ["laughed", "emphasized"]
        );
        assert!(!removed);
        // Removing a type that is not there changes nothing.
        let (v, removed) =
            standing_reactions(vec![r(2000, false, Some(1)), r(3001, true, Some(1))]);
        assert_eq!(v.len(), 1);
        assert!(!removed);
    }

    #[test]
    fn file_states() {
        assert_eq!(file_state(Some("not_in_backup"), None, &[]), "missing");
        assert_eq!(file_state(Some("no_path"), None, &[]), "no_path");
        assert_eq!(file_state(None, None, &[]), "not_copied");
        assert_eq!(
            file_state(None, Some("x"), &["done".into(), "pending".into()]),
            "derivative_pending"
        );
        assert_eq!(
            file_state(None, Some("x"), &["failed".into(), "pending".into()]),
            "derivative_failed"
        );
        assert_eq!(file_state(None, Some("x"), &["done".into()]), "ok");
    }
}
