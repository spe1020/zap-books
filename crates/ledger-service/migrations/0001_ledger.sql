-- The ledger service's store. Event JSON is kept as text, exactly as
-- signed, so it is served byte for byte; jsonb would reorder keys.

create table businesses (
    id text primary key,
    seq bigint not null,
    head text not null,
    epoch bigint not null,
    -- BusinessData after the head acceptance.
    snapshot jsonb not null,
    -- MembershipState after the head acceptance.
    membership jsonb not null,
    created_at timestamptz not null default now()
);

-- The books: append-only, one row per acceptance.
create table acceptances (
    business_id text not null references businesses (id),
    seq bigint not null,
    event_id text not null unique,
    proposal_id text not null,
    event text not null,
    primary key (business_id, seq)
);

-- Every proposal's acknowledgment, keyed by its event id: resending the same
-- event returns this exact text.
create table proposals (
    event_id text primary key,
    business_id text not null,
    author text not null,
    status text not null check (status in ('accepted', 'rejected')),
    ack text not null,
    created_at timestamptz not null default now()
);

-- The seq of the last acceptance that changed each record, for baseSeq
-- conflicts.
create table record_versions (
    business_id text not null references businesses (id),
    record_type text not null,
    record_id text not null,
    seq bigint not null,
    primary key (business_id, record_type, record_id)
);

create table members (
    business_id text not null references businesses (id),
    pubkey text not null,
    role text not null,
    history text not null,
    joined_epoch bigint not null,
    primary key (business_id, pubkey)
);

-- Owner-level changes waiting out their 48 hours (used from PR 3).
create table pending_changes (
    business_id text not null references businesses (id),
    proposal_id text not null,
    proposer text not null,
    change jsonb not null,
    effective_at bigint not null,
    primary key (business_id, proposal_id)
);

-- Epoch keys. The secret is NIP-44 encrypted from the service key to itself.
create table epochs (
    business_id text not null references businesses (id),
    epoch bigint not null,
    public_key text not null,
    secret_sealed text not null,
    primary key (business_id, epoch)
);

create table epoch_deliveries (
    business_id text not null references businesses (id),
    epoch bigint not null,
    member text not null,
    event text not null,
    primary key (business_id, epoch, member)
);
create index epoch_deliveries_member on epoch_deliveries (member);

-- Events to publish to relays in stage 4.
create table outbox (
    id bigserial primary key,
    event_id text not null,
    event text not null,
    created_at timestamptz not null default now(),
    published_at timestamptz
);

-- Rejections: signed, sent to the proposer, never part of the chain.
create table audit (
    id bigserial primary key,
    business_id text not null,
    proposal_id text not null,
    author text not null,
    code text not null,
    message text not null,
    rejection text not null,
    created_at timestamptz not null default now()
);
create index audit_author on audit (author, id);

-- NIP-98 auth events already used, so none is accepted twice.
create table nip98_seen (
    event_id text primary key,
    seen_at bigint not null
);
