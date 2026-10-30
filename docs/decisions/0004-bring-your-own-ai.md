# Decision 0004: Bring your own AI

**Status:** Accepted (2026-10-29)
**Save as:** `docs/decisions/0004-bring-your-own-ai.md`

## Context

Zap Books uses a local model for receipt reading (decision 0002). People will want to use AI they already run: a different local model, a model on another machine, or an AI agent they already work with. The concept plan's principles still hold: AI reduces work, suggestions show their evidence, the user approves, and nothing leaves the user's machine without an explicit choice.

## Decision

Support two levels of bring-your-own AI.

### Level 1: bring your own model (receipt reading)

- The extractor can talk to any **OpenAI-compatible chat endpoint** (LM Studio, llama.cpp server, vLLM, Ollama's compatible API), as well as Ollama's native API.
- **Local stays the default.** A model address that isn't on this computer is refused unless the user starts the extractor with an explicit flag (for example `--allow-remote-model`). When a remote model is in use:
  - the Review screen says so on every reading, naming the host;
  - the extractor's health endpoint reports it, so the app never says "nothing is sent anywhere else".
- **Every safety rule still applies.** Each answer goes through the same Rust checks: evidence grounding, arithmetic, refund and savings rules. A weaker model produces more "unclear", never a different standard.
- The benchmark tool works against any configured endpoint, so a user can measure their model on their own receipts before relying on it.

### Level 2: bring your own agent (a member of the business)

- **An AI agent joins a business like a person:** it has its own Nostr key, and an owner adds its npub with the new **assistant** role and a history choice.
- **The agent's key is held by the user's harness,** never by Zap Books. Every proposal it makes is signed by its own key, so its work is always attributable.
- **Removing the agent works like removing anyone:** it's refused immediately, and the business key rotates.
- **An MCP server** (later) exposes Zap Books tools to the user's agent: read drafts and reports, create drafts, attach suggestions. It signs with the agent's key and uses the same service API as everyone else.

### The assistant role

| Can                                                          | Can't                                       |
| ------------------------------------------------------------ | ------------------------------------------- |
| Read the books (receives epoch keys, per its history choice) | Approve, correct, or import                 |
| Create drafts, and edit **drafts it created**                | Edit drafts created by people               |
| Attach **suggestions** to any draft                          | Change settings, rules or membership        |
| Submit receipts                                              | Veto, recover, or see pending owner changes |

- **Suggestions are a new command** (`Suggest`). A suggestion is a record attached to a draft: proposed category, purpose or note, with its evidence and the agent's key. It **never changes the draft's fields.** A person sees it on the Review screen and accepts or ignores it, exactly like a receipt reading.
- **Approval always stays with a person.** That's enforced by the service's permission check and the signature on every proposal, not by the interface.
- A draft created by an assistant shows that it came from one.

## Risks and how they're handled

- **Data leaves the user's control if the agent runs in the cloud.** An agent with read access can see the whole book. The member screen says so plainly when adding an assistant, and the user chooses. The history choice can limit what an agent can read.
- **Prompt injection.** Receipts and descriptions are untrusted text, and an agent can be tricked. The role is the defense: an assistant can create drafts and suggestions, but it can't approve, so nothing reaches the books without a person.
- **A runaway agent.** The per-key rate limits apply, and the owner can remove it at any time.
- **A stolen agent key** is handled like any member key: revoke it, and the business key rotates.

## What happens when

- **Now (before stage 2 PR 4):** reserve the **assistant** role and the **`Suggest`** command in `crates/protocol`. That covers the enum variants, the permission rules in the table above, the "edit only drafts it created" rule, and a suggestion record type in `ledger-core` (parity rule). Add tests for every permission. The browser's member screens (PR 4) offer the assistant role, but suggestions don't appear in the UI yet.
- **After MVP:**
  - showing suggestions on the Review screen;
  - the MCP server;
  - Level 1's OpenAI-compatible endpoint support and the remote-model flag.

## Options considered and set aside

- **The agent acts with a person's key.** That's simpler, but it's indistinguishable from the person, can't be limited, and can't be removed without the person rotating their identity.
- **An assistant that can approve below a set amount.** That's convenient, but a single prompt-injected receipt could post an expense. It could be revisited later, together with maker-checker limits.
- **Model-only integration, without agents.** It's safe but limiting, since agents are where most of the accounting value is (decision 0002, "Direction for AI work").
