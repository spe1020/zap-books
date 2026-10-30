/**
 * The ledger protocol, from `crates/protocol` compiled to WebAssembly
 * (`crates/protocol-wasm`, built by `npm run wasm:build`). Every security
 * rule the browser applies (chain checks, signatures, opening acceptances,
 * membership) runs here, never in TypeScript.
 */
import init, * as wasm from "../../crates/protocol-wasm/pkg/protocol_wasm.js";
import wasmUrl from "../../crates/protocol-wasm/pkg/protocol_wasm_bg.wasm?url";
import type { BusinessData } from "../domain/types";

let ready: Promise<unknown> | undefined;
/** Loads the WebAssembly module once. */
export function loadProtocol() {
  ready ??= init({ module_or_path: wasmUrl });
  return ready;
}

/** A signed Nostr event (NIP-01). */
export interface NostrEvent {
  id: string;
  pubkey: string;
  created_at: number;
  kind: number;
  tags: string[][];
  content: string;
  sig: string;
}

/** Where a verified chain ends: what the browser stores. */
export interface Head {
  seq: number;
  id: string;
  epoch: number;
  /** The key that must sign the next acceptance. */
  service: string;
}

export interface AcceptanceHeader {
  business: string;
  seq: number;
  prev: string | null;
  proposal: string;
  epoch: number;
  nextService: string | null;
  createdAt: number;
}

/** An opened acceptance. `content` is the protocol's `AcceptanceContent`. */
export interface Opened {
  header: AcceptanceHeader;
  content: {
    proposal: NostrEvent;
    payload: string;
    outcome: { type: string; effectiveAt?: number };
    [key: string]: unknown;
  };
}

/** The protocol's `MembershipState`, as stored. */
export interface Membership {
  business: string;
  members: Record<
    string,
    { role: string; history: string; joinedEpoch: number }
  >;
  recoveryKey: string | null;
  service: string;
  epoch: number;
  pending: unknown[];
  assistantDrafts?: Record<string, string>;
}

function call<T>(fn: (request: string) => string, request: unknown): T {
  return JSON.parse(fn(JSON.stringify(request))) as T;
}

export const protocol = {
  buildProposal: (r: {
    business: string;
    baseSeq?: number;
    command: unknown;
  }) =>
    call<{ kind: number; plaintext: string; tags: string[][] }>(
      wasm.buildProposal,
      r,
    ),
  checkSignedProposal: (r: {
    event: NostrEvent;
    plaintext: string;
    author: string;
  }) =>
    call<{
      id: string;
      business: string;
      command: string;
      baseSeq: number | null;
    }>(wasm.checkSignedProposal, r),
  verifyChain: (r: {
    business: string;
    service: string;
    after?: Head;
    acceptances: NostrEvent[];
  }) => call<Head | null>(wasm.verifyChain, r),
  openAcceptance: (r: {
    event: NostrEvent;
    epoch: number;
    epochSecret: string;
    service: string;
    business: string;
  }) => call<Opened>(wasm.openAcceptance, r),
  followMembership: (r: { state?: Membership; acceptance: Opened }) =>
    call<Membership>(wasm.followMembership, r),
  applyRecordChanges: (r: { snapshot?: BusinessData; acceptance: Opened }) =>
    call<BusinessData>(wasm.applyRecordChanges, r),
  parseDelivery: (r: { event: NostrEvent; member: string; service: string }) =>
    call<{ business: string; epoch: number }>(wasm.parseDelivery, r),
  openDelivery: (r: {
    event: NostrEvent;
    member: string;
    service: string;
    plaintext: string;
  }) =>
    call<{ business: string; epoch: number; secret: string }>(
      wasm.openDelivery,
      r,
    ),
  parseRejection: (r: { event: NostrEvent; member: string; service: string }) =>
    call<{ proposal: string }>(wasm.parseRejection, r),
  openRejection: (r: {
    event: NostrEvent;
    member: string;
    service: string;
    plaintext: string;
  }) =>
    call<{
      proposal: string;
      code: "rejected" | "tooLarge";
      message: string;
      size?: number;
      limit?: number;
    }>(wasm.openRejection, r),
  limits: () =>
    JSON.parse(wasm.limits()) as {
      importBatchStart: number;
      maxEventBytes: number;
    },
  nextBatch: (r: { rows: number; size: number; limit: number }) =>
    call<number>(wasm.nextBatch, r),
  generateRecoveryKeypair: () =>
    JSON.parse(wasm.generateRecoveryKeypair()) as {
      secretKey: string;
      publicKey: string;
    },
};
