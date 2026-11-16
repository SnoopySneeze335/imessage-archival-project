-- archive.db, schema version 4: retrying derivatives after a decoder fix.
-- See docs/decisions/0006-decoder-workarounds.md.
--
-- `decoder` is the version of the input handling (media::input::DECODER)
-- that last tried to make the derivative. A failed derivative from an older
-- version is set back to pending by the next run, so a fix reaches files
-- that failed before it without redoing anything that worked. Every row
-- that exists now was tried by version 1.
ALTER TABLE derivative ADD COLUMN decoder INTEGER NOT NULL DEFAULT 1;
