//! The demo archive covers every kind and state the view knows (decision
//! 0010): its "All message kinds" conversations hold one example of each,
//! after a label message naming it, and check-render reports only counts and
//! GUIDs.

mod common;

use std::collections::{BTreeSet, HashMap};

use fixture_gen::{Variant, generate_variant, scenario};
use imessage_ap::{
    archive, attachments, derive,
    dirs::Dirs,
    ingest,
    view::{self, FEATURES, OUTSIDE_CHATS, PENDING, TRANSIENT, check},
};
use rusqlite::Connection;

const KINDS_CHATS: [&str; 2] = ["All message kinds", "All message kinds (group)"];

fn demo_archive(tmp: &tempfile::TempDir) -> Dirs {
    let backup = tmp.path().join("backup");
    generate_variant(&backup, Variant::Demo { large: false }).unwrap();
    let dirs = Dirs::under(&tmp.path().join("data"));
    ingest::run(&backup, &dirs).unwrap();
    attachments::run(&backup, &dirs, &attachments::Options::default()).unwrap();
    derive::run(&dirs, &derive::Options::default()).unwrap();
    dirs
}

/// The label a "▶ <label>: …" message names.
fn named_label(item: &view::Item) -> Option<String> {
    match &item.body {
        view::Body::Message { text: Some(t), .. } if item.from_me => t
            .strip_prefix("▶ ")
            .and_then(|r| r.split_once(": "))
            .map(|(l, _)| l.to_owned()),
        _ => None,
    }
}

#[test]
fn every_kind_has_an_example_after_its_label() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = demo_archive(&tmp);
    let db = archive::open_read_only(&dirs).unwrap();
    let chats: HashMap<i64, String> = db
        .prepare("SELECT id, display_name FROM chat WHERE display_name IN (?1, ?2)")
        .unwrap()
        .query_map(KINDS_CHATS, |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(chats.len(), 2);

    let timeline = view::Timeline::build(&db).unwrap();
    let known: BTreeSet<&str> = FEATURES.iter().map(|(l, _)| *l).collect();
    let mut in_chats: BTreeSet<&'static str> = BTreeSet::new();
    let mut labels_seen = 0;
    for (chat, name) in &chats {
        let items = view::load(&db, &timeline.chats[chat]).unwrap();
        for (i, item) in items.iter().enumerate() {
            in_chats.extend(item.features.iter().copied());
            if let Some(label) = named_label(item) {
                labels_seen += 1;
                assert!(
                    known.contains(label.as_str()),
                    "{name}: label {label:?} is not in FEATURES"
                );
                let next = items
                    .get(i + 1)
                    .unwrap_or_else(|| panic!("{name}: nothing after {label}"));
                assert!(
                    next.features.iter().any(|f| *f == label),
                    "{name}: the item after the {label:?} label has {:?}",
                    next.features
                );
            }
        }
    }
    assert!(labels_seen > 40, "{labels_seen}");

    let report = check::run(&db, check::Options::default()).unwrap();
    assert_eq!(report.render_errors, 0);
    let missing: Vec<&str> = FEATURES
        .iter()
        .map(|(l, _)| *l)
        .filter(|l| !TRANSIENT.contains(l))
        .filter(|l| {
            if OUTSIDE_CHATS.contains(l) {
                report.features[*l] == 0
            } else {
                !in_chats.contains(l)
            }
        })
        .collect();
    assert!(
        missing.is_empty(),
        "kinds with no example in the demo's \"All message kinds\" chats: {missing:?}. \
         Add one to fixtures/generator/src/scenario/demo.rs, or (for a kind not built yet) \
         list it in view::PENDING instead of FEATURES."
    );
    // Pending kinds are not produced yet.
    for (p, _) in PENDING {
        assert!(!report.features.contains_key(*p), "{p}");
    }
}

#[test]
fn check_render_prints_counts_and_guids_only() {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = demo_archive(&tmp);
    let db = archive::open_read_only(&dirs).unwrap();
    let report = check::run(&db, check::Options { guids: usize::MAX }).unwrap();
    let table = check::render(&report);
    let json = serde_json::to_string(&report).unwrap();

    let s = scenario::build_demo(false);
    let mut words: BTreeSet<String> = BTreeSet::new();
    for m in &s.messages {
        for t in m.text.iter().chain(&m.subject) {
            let t = t.replace('\u{FFFC}', "");
            if t.trim().chars().count() >= 6 {
                words.insert(t.trim().to_owned());
            }
        }
    }
    for h in &s.handles {
        words.insert(h.id.to_owned());
    }
    for c in &s.contacts {
        words.extend(c.first.iter().chain(&c.last).map(|n| (*n).to_owned()));
    }
    for a in &s.attachments {
        words.insert(a.transfer_name.clone());
    }
    for w in &words {
        assert!(!table.contains(w.as_str()), "table shows {w:?}");
        assert!(!json.contains(w.as_str()), "JSON shows {w:?}");
    }

    // Counts that must match the archive.
    let conn = Connection::open(dirs.archive_db()).unwrap();
    assert_eq!(
        report.messages,
        common::count(&conn, "SELECT COUNT(*) FROM message")
    );
    assert_eq!(report.items + report.reactions, report.messages);
    assert_eq!(report.problems["problem:orphan"].count, 1);

    // The details add up to the archive's own totals, and the demo holds an
    // example of each part.
    let d = &report.details;
    let r = &d.replies;
    assert_eq!(
        r.on_items + r.on_reactions + r.self_reference,
        r.with_reply_to
    );
    assert_eq!(
        r.with_reply_to,
        common::count(
            &conn,
            "SELECT COUNT(*) FROM message WHERE reply_to_guid IS NOT NULL"
        )
    );
    assert!(r.on_reactions >= 1 && r.target_missing >= 1);
    let c = &d.conversations;
    assert_eq!(c.listed + c.no_messages + c.reactions_only, c.chats);
    assert_eq!(c.listed, report.conversations);
    assert!(c.no_messages >= 1 && c.reactions_only >= 1);
    let a = &d.attachments;
    let per_file: usize = [&a.on_items_visible, &a.on_items_hidden, &a.on_reactions]
        .iter()
        .flat_map(|m| m.values())
        .sum();
    assert_eq!(a.ownerless + per_file, a.records);
    assert_eq!(
        a.missing_on_messages,
        [&a.on_items_visible, &a.on_items_hidden, &a.on_reactions]
            .iter()
            .filter_map(|m| m.get("missing"))
            .sum::<usize>()
    );
    assert!(a.unrecorded_placeholders >= 2);
    let au = &d.audio_messages;
    let other: usize = au.other_media.values().sum();
    assert_eq!(
        au.unsent + au.voice + other + au.hidden_only + au.no_record + au.no_attachment,
        au.total
    );
    assert!(au.no_record >= 1 && other >= 1);
    let rx = &d.reactions;
    let f = &rx.folded;
    assert_eq!(
        f.adds + f.removals + f.unknown_kind + rx.target_missing,
        rx.rows
    );
    assert_eq!(rx.rows, report.reactions);
    assert!(f.removals_matched >= 1 && f.removals_without_add >= 1 && f.removals_other_kind >= 1);
    let empty: usize = [
        "problem:empty:no_body",
        "problem:empty:body_undecoded",
        "problem:empty:blank_text",
        "problem:empty:only_hidden_attachments",
    ]
    .iter()
    .map(|l| report.features[*l])
    .sum();
    assert_eq!(empty, report.features["problem:empty"]);
    assert_eq!(report.problems["problem:reaction_target_missing"].count, 1);
}

#[test]
fn reactions_follow_removals_and_replacements() {
    let tmp = tempfile::tempdir().unwrap();
    let backup = tmp.path().join("backup");
    generate_variant(&backup, Variant::V1).unwrap();
    let dirs = Dirs::under(&tmp.path().join("data"));
    ingest::run(&backup, &dirs).unwrap();
    let db = archive::open_read_only(&dirs).unwrap();
    // In v1, Avery loves "Yes — 10am works…", takes it back, then likes it.
    let s = scenario::build();
    let target = s
        .messages
        .iter()
        .find(|m| {
            m.text
                .as_deref()
                .is_some_and(|t| t.starts_with("Yes — 10am"))
        })
        .unwrap();
    let id: i64 = db
        .query_row(
            "SELECT id FROM message WHERE guid = ?1",
            [&target.guid],
            |r| r.get(0),
        )
        .unwrap();
    let item = view::load(&db, &[id]).unwrap().remove(0);
    let kinds: Vec<&str> = item.reactions.iter().map(|r| r.kind).collect();
    assert_eq!(kinds, ["liked"]);
    assert!(item.features.contains(&"reaction:removed"));
    assert!(item.features.contains(&"message") && item.features.contains(&"text"));
}
