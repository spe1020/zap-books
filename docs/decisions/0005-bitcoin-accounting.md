# Decision 0005: Bitcoin accounting

**Status:** Accepted (2026-11-01)
**Save as:** `docs/decisions/0005-bitcoin-accounting.md`

## Context

Zap Books' most likely early users are bitcoin-native businesses: people who earn, hold and spend bitcoin and already use Nostr. Today the ledger supports USD, EUR and GBP with two decimal places, never converts between currencies, and records expenses only.

Bitcoin accounting raises two separate questions:

1. **Recording** amounts in bitcoin.
2. **Valuation and tax.** In the US, bitcoin is generally treated as property, so spending it is also a disposal that can create a gain or loss. Rules differ by country. Zap Books keeps the records; it never gives tax advice.

## Decision

### Principles

- **Sats are first-class, never silently mixed with dollars.** Reports never add bitcoin and dollar amounts together.
- **A dollar value is recorded, never assumed.** Every bitcoin entry carries the fiat value at the time of the transaction and where that value came from. Reports in dollars use the recorded values, never a live price.
- **The model stays deterministic and verifiable.** No live price lookup ever happens inside `ledger-core`. A value comes in as data on the command, and is checked, signed and audited like everything else.
- **Privacy in price lookups.** When the app suggests a price, it fetches a general price series (for example daily prices), never per-transaction requests that would reveal amounts or timing.

### Phase 1: bitcoin in the ledger (target: the beta)

- **Currency `BTC`, with sats as the smallest unit.** Add a per-currency precision table: USD, EUR and GBP use 2 decimals, and BTC uses 8 (1 BTC = 100,000,000 sats). Money parsing and display take the currency's precision. Sats are the default display; BTC is a display option. The existing range limit covers more than all 21 million BTC in sats.
- **Wallet accounts:** a new account kind, `wallet`, for on-chain or Lightning holdings in BTC. It behaves like `bank` for balances.
- **Recorded fiat value:** every journal with currency BTC carries a `fiatValue`:
  - the reporting currency and its amount in minor units;
  - the rate used;
  - the source: `manual`, `price-series` or `exchange`;
  - the time it applies to.

  The value is entered or confirmed by a person. `ledger-core` checks its shape and that amount equals sats × rate within rounding.

- **Exchange transfers:** a new transfer type, `exchange`, for buying or selling bitcoin between a dollar account and a wallet. It's a two-currency entry with both amounts and the implied rate. It's the only place two currencies meet, and it's always explicit.
- **Lightning amounts:** whole sats are stored. Millisat amounts are rounded to the nearest sat, and the original is kept as evidence.
- **Reports:**
  - bitcoin balances in sats;
  - spending in sats;
  - a clearly labeled "dollar value at time of transaction" view built only from recorded values;
  - exports with both.

### Phase 2: cost basis and gains (after the beta opens)

- **Acquisitions become lots,** with a date, sats and fiat cost.
- **Spending or selling bitcoin consumes lots:** first in, first out by default, or specific lots when the user chooses. Each disposal records its realized gain or loss.
- **Basis is tracked per wallet.** Recent US guidance points that way, and it's the more conservative design; accountants confirm what applies.
- **A gains report and export** for the accountant.

### Phase 3: Lightning and income

- **Income,** as its own feature, including zaps received.
- **Imports:** Lightning payment history from a connected wallet (NIP-47, Nostr Wallet Connect, read-only) and CSV imports from common exchanges and wallets, as payment evidence matched to receipts, the way bank CSVs work today.

## What this changes

- `ledger-core` and the TypeScript domain: precision per currency, the BTC currency, the wallet kind, `fiatValue`, and the exchange transfer type. These are parity changes: TypeScript first, regenerate fixtures, then Rust.
- The protocol: commands carry `fiatValue`. Permissions are unchanged.
- The browser: entering amounts in sats or BTC, confirming a fiat value with an optional suggested price, a wallet account setup, an exchange form, and reports and exports.

## Not decided here

- Which price series to offer for suggestions, and whether to offer one at all in phase 1, rather than manual entry only.
- Lot methods beyond FIFO and specific identification.
- Non-US tax treatments.

## Options considered and set aside

- **Converting everything to dollars at entry:** this loses the sats, which is the record this audience cares about most.
- **Live prices in reports:** results change with the market and can't be reproduced or audited.
- **Treating bitcoin as an ordinary foreign currency:** this misses the property and gains treatment that accountants need.
