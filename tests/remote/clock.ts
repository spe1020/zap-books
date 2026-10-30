/**
 * Moving the test service's clock (its `e2e-clock` build), so a 48-hour
 * wait can pass in a test. Pages and the harness must move with it: the
 * service refuses NIP-98 events more than a minute off its own time.
 */
import { writeFileSync } from "node:fs";
import type { BrowserContext } from "@playwright/test";
import { CLOCK_FILE } from "./keys";

let offset = 0;

/** Milliseconds the service's clock is ahead of the real one. */
export const clockOffset = () => offset;

/** Now, as the service sees it, in milliseconds. */
export const serviceNow = () => Date.now() + offset;

/** Moves the service's clock to `ms` ahead of the real one. */
export function setClockOffset(ms: number) {
  offset = ms;
  writeFileSync(CLOCK_FILE, String(Math.round(ms)));
}

/** Gives a new context's pages the service's time. Call before any page. */
export async function followClock(context: BrowserContext) {
  if (offset) await context.clock.install({ time: serviceNow() });
}
