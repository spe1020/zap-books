/**
 * The app's Content-Security-Policy, delivered as a `<meta>` tag by the
 * plugin in `vite.config.ts`. Scripts and styles come only from the app's
 * own origin; network requests go only there, to the receipt extractor
 * (none in a demo-only build) and, when one is configured, to exactly the
 * ledger service's origin;
 * receipts are shown from `blob:` URLs, PDFs through `<object>` (which Chrome
 * frames). Nothing else may be framed, and nothing posted or used as a base
 * URL.
 *
 * `dev` adds what Vite's dev server needs: its inline React refresh script,
 * the `<style>` tags it injects, and its WebSocket.
 *
 * With a service, `'wasm-unsafe-eval'` lets the app compile its own
 * WebAssembly (the protocol, `crates/protocol-wasm`). It allows no `eval`.
 */
export function contentSecurityPolicy({
  dev,
  extractorUrl,
  serviceUrl,
}: {
  dev: boolean;
  extractorUrl?: string;
  serviceUrl?: string;
}): string {
  const extractor = extractorUrl ? ` ${new URL(extractorUrl).origin}` : "";
  const service = serviceUrl ? ` ${new URL(serviceUrl).origin}` : "";
  const inline = dev ? " 'unsafe-inline'" : "";
  const wasm = serviceUrl ? " 'wasm-unsafe-eval'" : "";
  return [
    "default-src 'self'",
    `script-src 'self'${wasm}${inline}`,
    `style-src 'self'${inline}`,
    "img-src 'self' blob: data:",
    "object-src blob:",
    `connect-src 'self'${extractor}${service}${dev ? " ws:" : ""}`,
    // Chrome shows a PDF in <object> by framing its blob: URL.
    "frame-src blob:",
    "base-uri 'none'",
    "form-action 'none'",
  ].join("; ");
}
