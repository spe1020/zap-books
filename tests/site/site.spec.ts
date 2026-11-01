import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { existsSync, readFileSync } from "node:fs";

// The site is built with tests/site/form-service.json. Its service is
// invented, and every request to it is stubbed by route interception.
const service = JSON.parse(
  readFileSync(new URL("./form-service.json", import.meta.url), "utf8"),
) as { name: string; endpoint: string; contactEmail: string };
const serviceOrigin = new URL(service.endpoint).origin;
const REPO = "https://github.com/spe1020/zap-books";
// An invented key: "npub1" and 58 characters of the bech32 alphabet.
const NPUB = `npub1${"q".repeat(29)}${"x".repeat(29)}`;

const widths = [
  { name: "phone", width: 360, height: 800 },
  { name: "desktop", width: 1280, height: 900 },
];

/** Every request the page makes, and every error it logs. */
function watch(page: Page) {
  const requests: string[] = [];
  const errors: string[] = [];
  page.on("request", (r) => requests.push(r.url()));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(e.message));
  return { requests, errors };
}

const openAllDetails = (page: Page) =>
  page
    .locator("details")
    .evaluateAll((all) =>
      all.forEach((d) => ((d as HTMLDetailsElement).open = true)),
    );

const beta = (page: Page) => page.locator("#beta");

for (const { name, width, height } of widths) {
  test.describe(`at ${width} px (${name})`, () => {
    test.use({ viewport: { width, height } });

    test("loads only its own files, with no errors or overflow", async ({
      page,
    }) => {
      const { requests, errors } = watch(page);
      await page.goto("/");
      await expect(
        page.getByRole("heading", {
          level: 1,
          name: "Clear books. Your business. Your control.",
        }),
      ).toBeVisible();
      await page.evaluate(() => document.fonts.ready);
      await openAllDetails(page);
      const overflow = await page.evaluate(() => {
        const doc = document.documentElement;
        const wide = [...document.querySelectorAll("body *")]
          .filter(
            (el) => el.getBoundingClientRect().right > doc.clientWidth + 1,
          )
          .map((el) => `${el.tagName.toLowerCase()}.${el.className}`);
        return { scroll: doc.scrollWidth, client: doc.clientWidth, wide };
      });
      expect(overflow.wide).toEqual([]);
      expect(overflow.scroll).toBeLessThanOrEqual(overflow.client);
      expect(errors).toEqual([]);
      for (const url of requests)
        expect(new URL(url).origin, url).toBe("http://127.0.0.1:4180");
    });

    test("passes the accessibility scan, panels open and closed", async ({
      page,
    }) => {
      await page.goto("/");
      const closed = await new AxeBuilder({ page }).analyze();
      expect(closed.violations).toEqual([]);
      await openAllDetails(page);
      const open = await new AxeBuilder({ page }).analyze();
      expect(open.violations).toEqual([]);
    });
  });
}

test("every link and anchor resolves", async ({ page, request }) => {
  await page.goto("/");
  const hrefs = await page
    .locator("a[href], link[href]")
    .evaluateAll((all) => all.map((a) => a.getAttribute("href")!));
  expect(hrefs.length).toBeGreaterThan(20);
  for (const href of new Set(hrefs)) {
    if (href.startsWith("#")) {
      await expect(page.locator(href), href).toHaveCount(1);
      continue;
    }
    const url = new URL(href, page.url());
    if (url.origin === "http://127.0.0.1:4180") {
      const res = await request.get(url.href);
      expect(res.status(), href).toBe(200);
      if (url.hash)
        expect(await res.text(), href).toContain(`id="${url.hash.slice(1)}"`);
      continue;
    }
    if (url.origin === "https://zapbooks.io") {
      // Canonical and Open Graph addresses: the same files, deployed.
      const res = await request.get(url.pathname);
      expect(res.status(), href).toBe(200);
      continue;
    }
    // The repository's own pages. CI doesn't reach GitHub, so a link to
    // one of its files is checked against this checkout instead.
    expect(href === REPO || href.startsWith(`${REPO}/`), href).toBe(true);
    const file = href.match(/\/blob\/business-finance-mvp\/(.+)$/)?.[1];
    if (file) expect(existsSync(file), href).toBe(true);
  }
});

test("the Content-Security-Policy allows only the form service", async ({
  page,
}) => {
  await page.goto("/");
  const policy = await page
    .locator('meta[http-equiv="Content-Security-Policy"]')
    .getAttribute("content");
  const directives = Object.fromEntries(
    policy!.split("; ").map((d) => {
      const [k, ...v] = d.split(" ");
      return [k, v];
    }),
  );
  expect(directives["default-src"]).toEqual(["'self'"]);
  expect(directives["script-src"]).toEqual(["'self'"]);
  expect(directives["style-src"]).toEqual(["'self'"]);
  expect(directives["connect-src"]).toEqual([serviceOrigin]);
  expect(directives["form-action"]).toEqual([serviceOrigin]);
  await expect(
    beta(page).getByText(`Requests are collected through ${service.name}.`),
  ).toBeVisible();
});

test("the email address is checked before anything is sent", async ({
  page,
}) => {
  const sent: string[] = [];
  await page.route(`${serviceOrigin}/**`, (route) => {
    sent.push(route.request().url());
    return route.fulfill({ status: 200, json: { ok: true } });
  });
  await page.goto("/#beta");
  const input = beta(page).getByLabel("Email address");
  const submit = beta(page).getByRole("button", { name: "Request an invite" });
  await submit.click();
  await expect(beta(page).getByText("Enter your email address.")).toBeVisible();
  await expect(input).toHaveAttribute("aria-invalid", "true");
  await expect(input).toBeFocused();
  await input.fill("owner@example");
  await expect(input).not.toHaveAttribute("aria-invalid");
  await submit.click();
  await expect(
    beta(page).getByText("Enter a valid email address, like you@business.com."),
  ).toBeVisible();
  expect(sent).toEqual([]);
});

test("a Nostr public key can be used instead, and is checked", async ({
  page,
}) => {
  const sent: string[] = [];
  await page.route(`${serviceOrigin}/**`, (route) => {
    sent.push(route.request().url());
    return route.fulfill({ status: 200, json: { ok: true } });
  });
  await page.goto("/#beta");
  await beta(page)
    .getByRole("button", { name: "Use a Nostr public key instead" })
    .click();
  const input = beta(page).getByLabel("Nostr public key (npub)");
  await expect(input).toHaveAttribute("type", "text");
  const submit = beta(page).getByRole("button", { name: "Request an invite" });
  await submit.click();
  await expect(
    beta(page).getByText("Enter your Nostr public key."),
  ).toBeVisible();
  await input.fill("npub1short");
  await submit.click();
  await expect(
    beta(page).getByText(
      "Enter a Nostr public key that starts with npub1 and is 63 characters long.",
    ),
  ).toBeVisible();
  await input.fill(`${NPUB.slice(0, -1)}b`);
  await submit.click();
  await expect(input).toHaveAttribute("aria-invalid", "true");
  expect(sent).toEqual([]);
  await beta(page)
    .getByRole("button", { name: "Use an email address instead" })
    .click();
  await expect(beta(page).getByLabel("Email address")).toHaveValue("");
  await expect(
    beta(page).getByRole("paragraph").filter({ hasText: "Enter" }),
  ).toHaveCount(0);
});

for (const mode of ["email", "npub"] as const) {
  test(`a request by ${mode} sends only it and which it is`, async ({
    page,
  }) => {
    const value = mode === "email" ? "owner@example.test" : NPUB;
    const bodies: (string | null)[] = [];
    await page.route(`${serviceOrigin}/**`, (route) => {
      bodies.push(route.request().postData());
      return route.fulfill({
        status: 200,
        headers: { "Access-Control-Allow-Origin": "*" },
        json: { ok: true },
      });
    });
    await page.goto("/#beta");
    if (mode === "npub")
      await beta(page)
        .getByRole("button", { name: "Use a Nostr public key instead" })
        .click();
    await beta(page)
      .getByLabel(
        mode === "email" ? "Email address" : "Nostr public key (npub)",
      )
      .fill(`  ${value} `);
    await beta(page).getByRole("button", { name: "Request an invite" }).click();
    const done = beta(page).getByRole("status");
    await expect(done).toContainText("Thanks, you're on the list.");
    await expect(done).toContainText(
      `We'll contact you at ${value} when invites go out.`,
    );
    await expect(
      beta(page).getByText("Thanks, you're on the list."),
    ).toBeFocused();
    await expect(page.locator("#beta-form")).toBeHidden();
    expect(bodies).toHaveLength(1);
    expect(Object.fromEntries(new URLSearchParams(bodies[0]!))).toEqual({
      contact: value,
      contact_type: mode,
    });
  });
}

for (const failure of ["an error status", "no connection"] as const) {
  test(`with ${failure}, the form says so and offers email`, async ({
    page,
  }) => {
    await page.route(`${serviceOrigin}/**`, (route) =>
      failure === "no connection"
        ? route.abort("connectionrefused")
        : route.fulfill({
            status: 500,
            headers: { "Access-Control-Allow-Origin": "*" },
            json: { error: "unavailable" },
          }),
    );
    await page.goto("/#beta");
    const input = beta(page).getByLabel("Email address");
    await input.fill("owner@example.test");
    await beta(page).getByRole("button", { name: "Request an invite" }).click();
    await expect(
      beta(page).getByText(
        "We couldn't send your request. Try again, or email us at",
      ),
    ).toBeVisible();
    await expect(
      beta(page).getByRole("link", { name: service.contactEmail }),
    ).toHaveAttribute("href", `mailto:${service.contactEmail}`);
    // Nothing is lost: the address is still there to try again.
    await expect(input).toHaveValue("owner@example.test");
    await expect(
      beta(page).getByRole("button", { name: "Request an invite" }),
    ).toBeEnabled();
    await expect(beta(page).getByRole("status")).toBeHidden();
  });
}

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });

  test("the form is an ordinary post to the form service", async ({ page }) => {
    let posted: string | null = null;
    await page.route(`${serviceOrigin}/**`, (route) => {
      posted = route.request().postData();
      return route.fulfill({
        status: 200,
        contentType: "text/html",
        body: "<!doctype html><title>Received</title><h1>Received</h1>",
      });
    });
    await page.goto("/#beta");
    await expect(
      beta(page).getByRole("button", {
        name: "Use a Nostr public key instead",
      }),
    ).toBeHidden();
    await beta(page).getByLabel("Email address").fill("owner@example.test");
    await beta(page).getByRole("button", { name: "Request an invite" }).click();
    await expect(page.getByRole("heading", { name: "Received" })).toBeVisible();
    expect(page.url()).toBe(service.endpoint);
    expect(Object.fromEntries(new URLSearchParams(posted!))).toEqual({
      contact_type: "email",
      contact: "owner@example.test",
    });
  });

  test("the panels and answers still open", async ({ page }) => {
    await page.goto("/");
    await page.getByText("Who can read my books?").click();
    await expect(
      page.getByText("The members you add, according to their role"),
    ).toBeVisible();
  });
});
