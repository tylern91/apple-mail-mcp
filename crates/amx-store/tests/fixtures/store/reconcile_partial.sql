-- A mailbox whose Envelope Index snapshot no longer contains a message the Tantivy index still
-- holds (rowid 2) -- the ordinary "message got deleted" case a full reconcile must catch. Named
-- for the Reconciler tests (parasxos #2): reconcile.rs's tests seed a Tantivy index with rowids
-- {1, 2} against this fixture, which defines only rowid 1, to prove rowid 2 gets deleted -- and,
-- separately, that a reconcile whose snapshot read cannot complete deletes neither.
INSERT INTO mailboxes (url, total_count, unread_count, deleted_count, change_identifier)
VALUES ('imap://user@example.com/INBOX', 1, 0, 0, NULL);

INSERT INTO subjects (subject) VALUES ('kept');

INSERT INTO messages (ROWID, subject, mailbox, deleted)
VALUES (1, 1, 1, 0);
