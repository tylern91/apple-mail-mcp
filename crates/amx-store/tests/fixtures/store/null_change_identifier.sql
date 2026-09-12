-- Real Apple Mail stores leave `change_identifier` NULL on every mailbox (observed: all 38
-- mailboxes across 5 accounts on the reference store) — Phase 2's sync planner cannot rely on
-- it to detect drift and must fall back to (total_count, unread_count, deleted_count).
INSERT INTO mailboxes (url, total_count, unread_count, deleted_count, change_identifier)
VALUES ('imap://user@example.com/INBOX', 120, 4, 0, NULL);
