import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { fillPage, formService } from "./build-pages.mjs";

const site = (name) =>
  readFileSync(new URL(`../site/${name}`, import.meta.url), "utf8");

const invented = {
  name: "Example Forms",
  endpoint: "https://forms.example.test/f/beta",
  contactEmail: "beta@example.test",
};

test("the site's form posts to Formspree", () => {
  expect(formService(site("form-service.json"))).toEqual({
    name: "Formspree",
    endpoint: "https://formspree.io/f/xdeewqvn",
    origin: "https://formspree.io",
    contactEmail: "hello@zapbooks.io",
  });
});

test("the site isn't built while the form service is a placeholder", () => {
  const placeholder = JSON.stringify({
    name: "REPLACE_WITH_FORM_SERVICE_NAME",
    endpoint: "https://REPLACE-WITH-FORM-ENDPOINT.invalid/",
    contactEmail: "REPLACE-WITH-CONTACT-EMAIL@example.invalid",
  });
  expect(() => formService(placeholder)).toThrow(
    /site\/form-service\.json: set "name"/,
  );
  for (const key of Object.keys(invented))
    expect(() =>
      formService(JSON.stringify({ ...invented, [key]: "" })),
    ).toThrow(`set "${key}"`);
  expect(() =>
    formService(
      JSON.stringify({ ...invented, endpoint: "http://forms.example.test/" }),
    ),
  ).toThrow("must be https");
  expect(() =>
    formService(JSON.stringify({ ...invented, contactEmail: "beta" })),
  ).toThrow("not an email");
});

test("the endpoint, its origin and the names are filled in, escaped", () => {
  const service = formService(
    JSON.stringify({ ...invented, name: 'Forms <"&>' }),
  );
  const page = fillPage(site("index.html"), service);
  expect(page).not.toMatch(/__FORM_/);
  expect(page).toContain(
    "connect-src https://forms.example.test; form-action https://forms.example.test;",
  );
  expect(page).toContain('action="https://forms.example.test/f/beta"');
  expect(page).toContain('data-contact-email="beta@example.test"');
  expect(page).toContain("collected through Forms &lt;&quot;&amp;&gt;.");
  expect(() => fillPage("__FORM_OTHER__", service)).toThrow("__FORM_OTHER__");
});

test("no inline script or style, and nothing loaded from another host", () => {
  const page = site("index.html");
  for (const script of page.match(/<script\b[^>]*>/g) ?? [])
    expect(script).toMatch(/\bsrc="[^"]+"/);
  expect(page).not.toMatch(/<style\b|\sstyle=|\son[a-z]+=/i);
  // src and <link> load; <a href> only navigates.
  const loads = [
    ...page.matchAll(/\ssrc="([^"]+)"/g),
    ...page.matchAll(/<link\b(?![^>]*rel="canonical")[^>]*\shref="([^"]+)"/g),
  ].map((m) => m[1]);
  expect(loads.length).toBeGreaterThan(5);
  for (const url of loads) expect(url, url).not.toMatch(/^(https?:)?\/\//);
  expect(site("styles.css")).not.toMatch(/url\(\s*["']?(https?:)?\/\//);
  expect(site("beta-form.js")).not.toMatch(/https?:\/\//);
});
