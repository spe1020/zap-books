-- Membership and keys (stage 2, PR 3).

-- A pending change keeps its proposal event and plaintext payload: the
-- acceptance that activates it (or records it lapsed) repeats both.
alter table pending_changes
    add column proposal text not null,
    add column payload text not null;
create index pending_changes_due on pending_changes (effective_at);
