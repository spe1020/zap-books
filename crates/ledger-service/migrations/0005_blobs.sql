-- Encrypted receipts (receipt storage, PR 1). The files are under
-- --blob-dir, one directory per business, each named by its hash; this is
-- their index. A blob is pending until an accepted record refers to it, and
-- a pending one is deleted after 24 hours. Referenced blobs are kept.
create table blobs (
    business_id text not null references businesses (id),
    hash text not null,
    size bigint not null check (size > 0),
    state text not null check (state in ('pending', 'referenced')),
    created_at bigint not null,
    primary key (business_id, hash)
);
create index blobs_pending on blobs (created_at) where state = 'pending';
