# 0010. The render model, check-render and the demo archive

- Status: accepted
- Date: 2026-11-16
- Milestone: 4a-1, Viewer

## Context

The v1 goal "every message renders without errors" has to be checked on
the real archive (371,159 messages) without anyone seeing its content. The
user cannot share screenshots or text from it. Reviews of the UI therefore
happen on invented data, and problems on real data come back as counts,
GUIDs and descriptions in words.

## Decision

**One conversion, on the server.** The browser never interprets archive
rows. `crates/imessage-ap/src/view` turns each message into the JSON the UI
draws:

- one body kind per item: `message`, `unsent`, `event` or `unknown`;
- its attachments with a media kind (`web_image`, `image`, `video`,
  `voice`, `audio`, `sticker`, `other`) and a file state (`ok`, `missing`,
  `no_path`, `derivative_failed`, `not_copied`, `derivative_pending`);
- the reactions still standing on it, after replacements and removals;
- flags: edited, recovered, reply, bubble or screen effect, service;
- every case it recognises named by a label from `view::FEATURES`, such as
  `media:image`, `file:missing`, `reaction:disliked` or `event:renamed`.

Reactions are not timeline items; they sit on their targets. A reaction
whose target is not in the archive is hidden in the UI, stays in archive.db,
and is counted. A message in no conversation is listed under "Unfiled".

**`imessage-ap check-render`** runs every message through the same code,
conversation by conversation, and prints:

- totals: messages, items drawn, reactions, conversations, render errors
  (messages that are neither an item nor a reaction; must be 0);
- items per label, every label listed even at 0;
- problems (`problem:*` labels) with their count and the first GUIDs
  (`--guids N`, `--all-guids`), never text, names or addresses.

`--json` gives the same as JSON. Its output is meant to be shared.

**The demo archive.** `make-fixture --variant demo [--large]` writes the v1
backup plus two "All message kinds" conversations, a 1:1 and a group. They
hold one example of every label, in a fixed order, each after a label
message from the owner, "▶ <label>: <what it shows>". `--large` adds a
conversation of 25,000 messages over six years. `docker run … demo` and
`scripts/demo.sh` generate it, import it, convert its files, set the
password `demo1234` and serve it, marked as the demo on the sign-in page.
Screenshots of it show only invented data.

**Coverage is a test.** `crates/imessage-ap/tests/demo.rs` fails when:

- a label in `FEATURES` has no example in the "All message kinds"
  conversations (labels in `OUTSIDE_CHATS` may be anywhere in the demo;
  `TRANSIENT` states exist only while `attachments` or `derive` has work
  left, so the demo cannot hold them);
- the item after a label message does not carry that label;
- a label message names a label `FEATURES` does not know;
- check-render's output contains any text, name, address or file name
  from the demo.

Kinds the view does not produce yet wait in `view::PENDING`, with the PR
that adds them (4b: link-preview content, edit history, reply quotes,
message parts; 4c: attachment URLs). A PR that builds one moves it into
`FEATURES` and adds its demo example in the same change, so the list
shrinks to zero by the end of Milestone 4.

**Details reconcile with `stats`.** check-render's `details` section ties
its counts to the archive's own:

- conversations listed vs chat rows, split into no messages and reactions
  only;
- attachments per file, by on-items-visible, on-items-hidden and
  on-reactions, with each file state;
- voice messages by what they hold;
- reaction rows as adds and removals, and what each removal matched;
- replies on items, on reactions and self-referencing.

The 4a-1 real check showed these were needed
(`docs/reference/README.md#render-check-baseline`). Empty messages carry
one reason (`problem:empty:*`). Attachment placeholders with no record are
drawn as `file:no_record`, not as empty. A reply whose original is not in
the archive is drawn with a "message not available" quote in 4b; the count
stays under problems so its GUIDs are listed.

## Consequences

- The UI can draw from an exhaustive set of kinds and states; anything new
  shows up first as a check-render count, not as a broken page.
- check-render on the real archive is how the v1 item "every message
  renders without errors" is checked: 0 render errors, and every problem
  count explained. It is run after each Milestone 4 PR.
- The demo grows with every kind the view learns, and the user can compare
  its counts with the real archive's kind by kind.
- The browser-side counterpart (drawing every message in the browser and
  counting failures) comes in Milestone 4d.
