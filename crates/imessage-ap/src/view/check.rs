//! `imessage-ap check-render`: runs every message in the archive through the
//! view and reports what it found as counts, with GUIDs for problems. It
//! never prints message text, names or addresses, so its output can be
//! shared (decision 0010).

use std::collections::{BTreeMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use super::{FEATURES, Item, Timeline, load};

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

    let timeline = for_each_item(db, |chat, item| {
        if !seen.insert(item.id) {
            several += 1;
            return Ok(());
        }
        if chat.is_none() {
            unfiled.push(item.guid.clone());
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
        let _ = writeln!(out, "  {label:<36}{:>9}  {desc}", n(p.count));
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
    out
}
