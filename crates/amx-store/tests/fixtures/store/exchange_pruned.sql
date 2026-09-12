-- An Exchange mailbox whose Envelope Index row survives even after Exchange prunes the
-- underlying .emlx from disk (umbrella Q4) — the message is still enumerable via `messages`,
-- but fetching its body finds nothing at the computed path. This is the fixture proving
-- BodyState::Unavailable(NotCachedLocally) rather than a parse failure or missing-row silence.
INSERT INTO mailboxes (url, total_count, unread_count, deleted_count, change_identifier)
VALUES ('exchange://work@example.com/INBOX', 50, 1, 0, NULL);

INSERT INTO messages (ROWID, subject, mailbox, deleted)
VALUES (1, 1, 1, 0);
