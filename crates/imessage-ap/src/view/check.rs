//! `imessage-ap check-render`: runs every message in the archive through the
//! view and reports what it found as counts, with GUIDs for problems. It
//! never prints message text, names or addresses, so its output can be
//! shared (decision 0010).

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use super::{
    Body, FEATURES, Item, ReactionTally, Timeline, all_reactions, file_state, load,
    standing_reactions,
};

/// What one run found.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Report {
    pub messages: usize,
    /// Messages drawn as timeline items (each counted once, even when it is
    /// in several conversations).
    pub items: usize,
    /// Reaction messages: drawn on their targets, not as items.
    pub reactions: usize,
    pub conversations: usize,
    /// Items in more than one conversation.
    pub items_in_several_conversations: usize,
    /// Items the view could not build at all. Must be 0.
    pub render_errors: usize,
    /// Items per label, every label of [`FEATURES`] included (0 when absent).
    pub features: BTreeMap<String, usize>,
    /// Problem label → count and the first GUIDs.
    pub problems: BTreeMap<String, Problem>,
    /// Counts that reconcile the totals above with `stats`.
    pub details: Details,
}

/// Breakdowns that tie check-render's counts to the archive's own.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Details {
    pub conversations: Conversations,
    pub attachments: Attachments,
    pub audio_messages: AudioMessages,
    pub reactions: Reactions,
    pub replies: Replies,
}

/// Chat rows and how many are listed.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Conversations {
    /// Rows in `chat`.
    pub chats: usize,
    /// Chats with at least one item: the conversation list.
    pub listed: usize,
    /// Chats no message belongs to (not listed).
    pub no_messages: usize,
    /// Chats whose only messages are reactions (not listed).
    pub reactions_only: usize,
}

/// Every attachment record, one count per file (not per message).
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Attachments {
    /// Rows in `attachment`.
    pub records: usize,
    /// Records no message refers to (iOS leftovers, decision 0003).
    pub ownerless: usize,
    /// Records more than one message refers to.
    pub on_several_messages: usize,
    /// On items and shown, by file state.
    pub on_items_visible: BTreeMap<String, usize>,
    /// On items but hidden by iOS, by file state.
    pub on_items_hidden: BTreeMap<String, usize>,
    /// On reaction messages (stickers placed on a message), by file state.
    pub on_reactions: BTreeMap<String, usize>,
    /// `missing = 'not_in_backup'` on any message (stats' "missing files on
    /// live messages").
    pub missing_on_messages: usize,
    pub no_path_on_messages: usize,
    /// Attachment placeholders in item text with no record at all.
    pub unrecorded_placeholders: usize,
}

/// Messages flagged `is_audio_message`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct AudioMessages {
    pub total: usize,
    /// Unsent: drawn as the unsent placeholder.
    pub unsent: usize,
    /// Items with a visible voice attachment (media:voice).
    pub voice: usize,
    /// Items whose visible attachments are another media kind, by kind.
    pub other_media: BTreeMap<String, usize>,
    /// Items whose only attachments are hidden.
    pub hidden_only: usize,
    /// Items with an attachment placeholder but no record (file:no_record).
    pub no_record: usize,
    /// Items with neither.
    pub no_attachment: usize,
}

/// Reaction rows, and what folding them per message did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Reactions {
    /// Rows in `reaction`.
    pub rows: usize,
    /// Rows whose target is not in the archive (hidden).
    pub target_missing: usize,
    /// On targets in the archive.
    #[serde(flatten)]
    pub folded: ReactionTally,
}

/// Messages with `reply_to_guid` (stats' `replies`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Replies {
    pub with_reply_to: usize,
    /// Items drawn as replies (the `reply` label).
    pub on_items: usize,
    /// Reactions inside a thread: drawn on their target, not as replies.
    pub on_reactions: usize,
    /// `reply_to_guid` equal to the message's own GUID (thread originators).
    pub self_reference: usize,
    /// Items whose original is not in the archive.
    pub target_missing: usize,
    /// Distinct originals not in the archive.
    pub target_missing_distinct: usize,
    /// Of those, originals a reaction also points at.
    pub target_missing_also_reacted_to: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Problem {
    pub count: usize,
    pub guids: Vec<String>,
}

/// Options for a run.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// GUIDs listed per problem (`usize::MAX` for all).
    pub guids: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { guids: 20 }
    }
}

/// Every item, in each conversation's order, then the unfiled ones, with the
/// conversation it was drawn in (`None` for unfiled). Each item is visited
/// once per conversation it is in.
pub fn for_each_item(
    db: &Connection,
    mut f: impl FnMut(Option<i64>, &Item) -> Result<()>,
) -> Result<Timeline> {
    let timeline = Timeline::build(db)?;
    for (chat, ids) in &timeline.chats {
        for item in load(db, ids)? {
            f(Some(*chat), &item)?;
        }
    }
    for item in load(db, &timeline.unfiled)? {
        f(None, &item)?;
    }
    Ok(timeline)
}

/// Runs the check over the whole archive.
pub fn run(db: &Connection, opts: Options) -> Result<Report> {
    let mut features: BTreeMap<String, usize> =
        FEATURES.iter().map(|(l, _)| ((*l).to_owned(), 0)).collect();
    let mut problems: BTreeMap<String, Problem> = BTreeMap::new();
    let mut seen: HashSet<i64> = HashSet::new();
    let mut several = 0usize;
    let mut add_problem = |name: &str, guid: &str| {
        let p = problems.entry(name.to_owned()).or_default();
        p.count += 1;
        if p.guids.len() < opts.guids {
            p.guids.push(guid.to_owned());
        }
    };
    let mut unfiled = Vec::new();
    let mut details = Details::default();
    let mut missing_targets: HashSet<String> = HashSet::new();

    let timeline = for_each_item(db, |chat, item| {
        if !seen.insert(item.id) {
            several += 1;
            return Ok(());
        }
        if chat.is_none() {
            unfiled.push(item.guid.clone());
        }
        let has = |l: &str| item.features.contains(&l);
        if has("reply") {
            details.replies.on_items += 1;
            if has("problem:reply_target_missing")
                && let Some(t) = &item.reply_to
            {
                details.replies.target_missing += 1;
                missing_targets.insert(t.clone());
            }
        }
        if let Body::Message {
            attachments,
            unrecorded_attachments,
            audio_message,
            ..
        } = &item.body
        {
            details.attachments.unrecorded_placeholders += unrecorded_attachments;
            if *audio_message {
                let a = &mut details.audio_messages;
                if attachments.iter().any(|x| x.media == "voice") {
                    a.voice += 1;
                } else if let Some(x) = attachments.first() {
                    *a.other_media.entry(x.media.to_owned()).or_default() += 1;
                } else if has("file:hidden") {
                    a.hidden_only += 1;
                } else if *unrecorded_attachments > 0 {
                    a.no_record += 1;
                } else {
                    a.no_attachment += 1;
                }
            }
        }
        for l in &item.features {
            *features.entry((*l).to_owned()).or_default() += 1;
            if l.starts_with("problem:") {
                add_problem(l, &item.guid);
            }
        }
        Ok(())
    })?;
    for guid in &unfiled {
        *features.entry("problem:orphan".into()).or_default() += 1;
        add_problem("problem:orphan", guid);
    }

    // Reactions whose target is not in the archive: hidden in the UI.
    let mut stmt = db.prepare(
        "SELECT m.guid FROM reaction r JOIN message m ON m.id = r.message_id
         WHERE NOT EXISTS (SELECT 1 FROM message t WHERE t.guid = r.target_guid)
         ORDER BY m.sent_at, m.id",
    )?;
    let orphaned: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for guid in &orphaned {
        *features
            .entry("problem:reaction_target_missing".into())
            .or_default() += 1;
        add_problem("problem:reaction_target_missing", guid);
    }

    let count = |sql: &str| {
        db.query_row(sql, [], |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
    };

    // Conversations.
    let c = &mut details.conversations;
    c.chats = count("SELECT COUNT(*) FROM chat")?;
    c.listed = timeline.chats.len();
    c.no_messages = count(
        "SELECT COUNT(*) FROM chat c WHERE NOT EXISTS (SELECT 1 FROM message_chat mc WHERE mc.chat_id = c.id)",
    )?;
    c.reactions_only = c.chats - c.listed - c.no_messages;

    // Attachments, one per record.
    let a = &mut details.attachments;
    a.records = count("SELECT COUNT(*) FROM attachment")?;
    a.ownerless = count(
        "SELECT COUNT(*) FROM attachment a WHERE NOT EXISTS (SELECT 1 FROM message_attachment ma WHERE ma.attachment_id = a.id)",
    )?;
    let mut stmt = db.prepare(
        "SELECT a.missing, a.sha256, a.hide_attachment,
                (SELECT group_concat(d.status) FROM derivative d WHERE d.sha256 = a.sha256),
                COUNT(*),
                MIN(EXISTS (SELECT 1 FROM reaction r WHERE r.message_id = ma.message_id))
         FROM attachment a JOIN message_attachment ma ON ma.attachment_id = a.id
         GROUP BY a.id",
    )?;
    let mut q = stmt.query([])?;
    while let Some(r) = q.next()? {
        let missing: Option<String> = r.get(0)?;
        let sha256: Option<String> = r.get(1)?;
        let hidden: bool = r.get(2)?;
        let derivs: Vec<String> = r
            .get::<_, Option<String>>(3)?
            .map(|s| s.split(',').map(str::to_owned).collect())
            .unwrap_or_default();
        let owners: i64 = r.get(4)?;
        let only_reactions: bool = r.get(5)?;
        let state = file_state(missing.as_deref(), sha256.as_deref(), &derivs).to_owned();
        a.on_several_messages += usize::from(owners > 1);
        a.missing_on_messages += usize::from(missing.as_deref() == Some("not_in_backup"));
        a.no_path_on_messages += usize::from(missing.as_deref() == Some("no_path"));
        let map = if only_reactions {
            &mut a.on_reactions
        } else if hidden {
            &mut a.on_items_hidden
        } else {
            &mut a.on_items_visible
        };
        *map.entry(state).or_default() += 1;
    }
    drop(q);
    drop(stmt);

    // Audio messages that are not items drawn as messages.
    details.audio_messages.total =
        count("SELECT COUNT(*) FROM message WHERE is_audio_message = 1")?;
    details.audio_messages.unsent =
        count("SELECT COUNT(*) FROM message WHERE is_audio_message = 1 AND is_unsent = 1")?;

    // Reactions, folded per target as the view does.
    let rx = &mut details.reactions;
    let mut reacted_missing: HashSet<String> = HashSet::new();
    for (target, exists, raw) in all_reactions(db)? {
        rx.rows += raw.len();
        if exists {
            rx.folded += standing_reactions(raw).1;
        } else {
            rx.target_missing += raw.len();
            reacted_missing.insert(target);
        }
    }

    // Replies.
    let rp = &mut details.replies;
    rp.with_reply_to = count("SELECT COUNT(*) FROM message WHERE reply_to_guid IS NOT NULL")?;
    rp.on_reactions = count(
        "SELECT COUNT(*) FROM message m JOIN reaction r ON r.message_id = m.id WHERE m.reply_to_guid IS NOT NULL",
    )?;
    rp.self_reference = count(
        "SELECT COUNT(*) FROM message m WHERE m.reply_to_guid = m.guid
         AND NOT EXISTS (SELECT 1 FROM reaction r WHERE r.message_id = m.id)",
    )?;
    rp.target_missing_distinct = missing_targets.len();
    rp.target_missing_also_reacted_to = missing_targets
        .iter()
        .filter(|t| reacted_missing.contains(*t))
        .count();
    let messages = count("SELECT COUNT(*) FROM message")?;
    let reactions = count("SELECT COUNT(*) FROM reaction")?;
    let items = seen.len();
    // Every message is either an item or a reaction; anything else could
    // not be built.
    let render_errors = messages.saturating_sub(items + reactions);
    Ok(Report {
        messages,
        items,
        reactions,
        conversations: timeline.chats.len(),
        items_in_several_conversations: several,
        render_errors,
        features,
        problems,
        details,
    })
}

/// The report as an aligned table.
pub fn render(r: &Report) -> String {
    use std::fmt::Write as _;
    let n = |v: usize| crate::stats::thousands(v as u64);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Render check (counts and GUIDs only; no message content)"
    );
    for (label, v) in [
        ("messages", r.messages),
        ("items drawn", r.items),
        ("reactions (on their messages)", r.reactions),
        ("conversations", r.conversations),
        (
            "items in several conversations",
            r.items_in_several_conversations,
        ),
        ("render errors", r.render_errors),
    ] {
        let _ = writeln!(out, "  {label:<34}{:>12}", n(v));
    }
    let _ = writeln!(out, "\nItems per kind and state");
    for (label, desc) in FEATURES {
        if label.starts_with("problem:") {
            continue;
        }
        let v = r.features.get(*label).copied().unwrap_or(0);
        let _ = writeln!(out, "  {label:<28}{:>12}  {desc}", n(v));
    }
    let _ = writeln!(out, "\nProblems");
    for (label, desc) in FEATURES.iter().filter(|(l, _)| l.starts_with("problem:")) {
        let p = r.problems.get(*label).cloned().unwrap_or_default();
        let _ = writeln!(out, "  {label:<40}{:>9}  {desc}", n(p.count));
        for g in &p.guids {
            let _ = writeln!(out, "      {g}");
        }
        if p.count > p.guids.len() {
            let _ = writeln!(
                out,
                "      … {} more (--guids N or --all-guids)",
                n(p.count - p.guids.len())
            );
        }
    }
    let _ = writeln!(out, "\nDetails (reconciling with `stats`)");
    if let Ok(v) = serde_json::to_value(&r.details) {
        flatten(&mut out, "", &v);
    }
    out
}

/// Writes every number in `v` as `  path.to.key   value`.
fn flatten(out: &mut String, prefix: &str, v: &serde_json::Value) {
    use std::fmt::Write as _;
    match v {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(out, &p, v);
            }
        }
        serde_json::Value::Number(n) => {
            let n = n
                .as_u64()
                .map_or_else(|| n.to_string(), crate::stats::thousands);
            let _ = writeln!(out, "  {prefix:<52}{n:>12}");
        }
        _ => {}
    }
}
