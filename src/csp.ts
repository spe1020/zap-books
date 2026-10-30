/**
 * The app's Content-Security-Policy, delivered as a `<meta>` tag by the
 * plugin in `vite.config.ts`. Scripts and styles come only from the app's
 * own origin; network requests go only there and to the receipt extractor;
 * receipts are shown from `blob:` URLs, PDFs through `<object>` (which Chrome
 * frames). Nothing else may be framed, and nothing posted or used as a base
 * URL.
 *
 * `dev` adds what Vite's dev server needs: its inline React refresh script,
 * the `<style>` tags it injects, and its WebSocket.
 */
export function contentSecurityPolicy({
  dev,
  extractorUrl,
}: {
  dev: boolean;
  extractorUrl: string;
}): string {
  const extractor = new URL(extractorUrl).origin;
  const inline = dev ? " 'unsafe-inline'" : "";
  return [
    "default-src 'self'",
    `script-src 'self'${inline}`,
    `style-src 'self'${inline}`,
    "img-src 'self' blob: data:",
    "object-src blob:",
    `connect-src 'self' ${extractor}${dev ? " ws:" : ""}`,
    // Chrome shows a PDF in <object> by framing its blob: URL.
    "frame-src blob:",
    "base-uri 'none'",
    "form-action 'none'",
  ].join("; ");
}
