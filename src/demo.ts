/**
 * A demo-only build (`VITE_DEMO_ONLY=true`), as hosted at zapbooks.io/demo/:
 * the local demo and nothing else. No sign-in, even when a ledger service is
 * configured, and no receipt reading, since a visitor's computer has no
 * extractor for it to ask.
 */
export const demoOnly = import.meta.env.VITE_DEMO_ONLY === "true";
