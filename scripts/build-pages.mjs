// Assembles zapbooks.io into one folder for GitHub Pages: site/ as it is,
// the brand files it uses from public/brand/, and the form service's
// settings from site/form-service.json filled into the page. Nothing is
// compiled. docs/landing-site.md has the steps.
//
//   node scripts/build-pages.mjs [--config <file>] [--out <dir>]

import {
  cpSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** The files the site uses from public/brand/. */
export const BRAND_FILES = [
  "zap-books-logo.svg",
  "zap-books-logo-dark-transparent.svg",
  "favicon.svg",
  "favicon-16.png",
  "favicon-32.png",
  "favicon-48.png",
  "app-icon-180.png",
  "app-icon-192.png",
  "app-icon-512.png",
  "Outfit-VariableFont_wght.woff2",
  "OFL-Outfit.txt",
];

/** site/ files that are settings, not pages. */
const NOT_PUBLISHED = new Set(["form-service.json"]);

/**
 * The form service's settings, checked. Throws while any is still a
 * placeholder, so a site with a form that can't work is never built.
 */
export function formService(json) {
  const { name, endpoint, contactEmail } = JSON.parse(json);
  for (const [key, value] of Object.entries({ name, endpoint, contactEmail }))
    if (typeof value !== "string" || !value.trim() || /replace/i.test(value))
      throw new Error(
        `site/form-service.json: set "${key}" (see docs/landing-site.md).`,
      );
  const url = new URL(endpoint);
  if (url.protocol !== "https:")
    throw new Error('site/form-service.json: "endpoint" must be https.');
  if (!/^[^\s@<>"]+@[^\s@<>"]+\.[^\s@<>"]+$/.test(contactEmail))
    throw new Error('site/form-service.json: "contactEmail" is not an email.');
  return {
    name: name.trim(),
    endpoint: url.href,
    origin: url.origin,
    contactEmail,
  };
}

const escapeHtml = (s) =>
  s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");

/** The page with the form service's settings in place. */
export function fillPage(html, service) {
  const values = {
    __FORM_ORIGIN__: service.origin,
    __FORM_ENDPOINT__: service.endpoint,
    __FORM_SERVICE__: service.name,
    __FORM_CONTACT_EMAIL__: service.contactEmail,
  };
  let out = html;
  for (const [placeholder, value] of Object.entries(values))
    out = out.replaceAll(placeholder, escapeHtml(value));
  const left = out.match(/__FORM_[A-Z_]+__/);
  if (left) throw new Error(`site/index.html: ${left[0]} has no value.`);
  return out;
}

export function buildPages({
  config = join(root, "site/form-service.json"),
  out = join(root, "dist-pages"),
} = {}) {
  const service = formService(readFileSync(config, "utf8"));
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  cpSync(join(root, "site"), out, {
    recursive: true,
    filter: (src) => !NOT_PUBLISHED.has(src.split(/[\\/]/).pop()),
  });
  const page = join(out, "index.html");
  writeFileSync(page, fillPage(readFileSync(page, "utf8"), service));
  mkdirSync(join(out, "brand"));
  for (const file of BRAND_FILES)
    cpSync(join(root, "public/brand", file), join(out, "brand", file));
  return out;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { values } = parseArgs({
    options: { config: { type: "string" }, out: { type: "string" } },
  });
  try {
    const out = buildPages({
      config: values.config && resolve(values.config),
      out: values.out && resolve(values.out),
    });
    console.log(`Built the site in ${out}`);
  } catch (e) {
    console.error(e instanceof Error ? e.message : e);
    process.exit(1);
  }
}
