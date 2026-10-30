-- Read sessions (stage 2, PR 4a): a token from POST /v1/session stands in
-- for a NIP-98 signature on reads for an hour. Only the token's SHA-256 is
-- kept, so a copy of this table opens no session.
create table sessions (
    token_hash text primary key,
    pubkey text not null,
    expires_at bigint not null
);
create index sessions_expiry on sessions (expires_at);
