import { test, expect, type Page, type Request } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { existsSync, readFileSync } from "node:fs";

// The site is built with its real settings, site/form-service.json:
// Formspree. Every request to Formspree is stubbed by route interception,
// so none leaves this computer.
const service = JSON.parse(
  readFileSync(
    new URL("../../site/form-service.json", import.meta.url),
    "utf8",
  ),
) as { name: string; endpoint: string; contactEmail: string };
const FORMSPREE = "https://formspree.io";
const serviceOrigin = new URL(service.endpoint).origin;
const SUBJECT = "Zap Books beta request";
// Formspree answers a request it accepts like this.
const accepted = {
  status: 200,
  headers: { "Access-Control-Allow-Origin": "*" },
  json: { ok: true, next: `${FORMSPREE}/thanks` },
};

/** The fields a request sent, form-encoded or multipart. */
async function fields(request: Request) {
  const body = new Response(request.postData() ?? "", {
    headers: { "Content-Type": request.headers()["content-type"] },
  });
  return Object.fromEntries(await body.formData());
}
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
          name: "Less bookkeeping. More control.",
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

test("planned AI is labeled as planned, wherever it's shown", async ({
  page,
}) => {
  await page.goto("/");
  const ai = page.locator("#ai");
  const local = ai.getByRole("article").filter({
    has: page.getByRole("heading", { name: "Run AI locally" }),
  });
  await expect(local).toContainText("Available now");
  await expect(local).toContainText("it isn't a chat assistant");
  const own = ai.getByRole("article").filter({
    has: page.getByRole("heading", { name: "Bring your own AI" }),
  });
  await expect(own).toContainText("Coming next · not available yet");
  await expect(own).toContainText("An API key isn't the same as local.");
  // The assistant preview is a concept, and says so before anything else.
  const concept = page.locator(".job-concept");
  await expect(concept).toContainText("Concept preview · coming next");
  await expect(concept).toContainText("Not a working feature.");
  // No provider's logo while no provider connection works: the only
  // images are the brand's own.
  const images = await page
    .locator("img")
    .evaluateAll((all) => all.map((i) => i.getAttribute("src")!));
  for (const src of images) expect(src, src).toMatch(/^brand\//);
  const text = await page.locator("main").innerText();
  // Claims the page must never make.
  for (const claim of [
    /zero[- ]knowledge/i,
    /unhackable/i,
    /only you can read/i,
    /never used for training/i,
  ])
    expect(text).not.toMatch(claim);
});

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

test("the Content-Security-Policy allows exactly Formspree", async ({
  page,
}) => {
  expect(serviceOrigin).toBe(FORMSPREE);
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
  expect(directives["connect-src"]).toEqual([FORMSPREE]);
  expect(directives["form-action"]).toEqual([FORMSPREE]);
  await expect(
    beta(page).getByText("Requests are collected through Formspree."),
  ).toBeVisible();
  const form = page.locator("#beta-form");
  await expect(form).toHaveAttribute("action", service.endpoint);
  await expect(form).toHaveAttribute("method", "POST");
});

test("the honeypot is in the form, but nobody can see or reach it", async ({
  page,
}) => {
  await page.goto("/#beta");
  const gotcha = page.locator('#beta-form input[name="_gotcha"]');
  await expect(gotcha).toHaveCount(1);
  await expect(gotcha).toHaveValue("");
  await expect(gotcha).toHaveAttribute("tabindex", "-1");
  // Clipped to a single pixel for sight, and out of the accessibility tree.
  const shown = await gotcha.evaluate((el) => {
    const hidden = el.closest('[aria-hidden="true"]');
    if (!hidden) return null;
    const box = hidden.getBoundingClientRect();
    const style = getComputedStyle(hidden);
    return {
      area: box.width * box.height,
      overflow: style.overflow,
      clip: style.clipPath,
    };
  });
  expect(shown).toEqual({
    area: 1,
    overflow: "hidden",
    clip: "inset(50%)",
  });
  await expect(beta(page).getByRole("textbox")).toHaveCount(1);
  // Tabbing from the address goes to the button, never to the honeypot.
  await beta(page).getByLabel("Email address").focus();
  await page.keyboard.press("Tab");
  await expect(
    beta(page).getByRole("button", { name: "Request an invite" }),
  ).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Shift+Tab");
  await expect(gotcha).not.toBeFocused();
});

test("the email address is checked before anything is sent", async ({
  page,
}) => {
  const sent: string[] = [];
  await page.route(`${FORMSPREE}/**`, (route) => {
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
  await page.route(`${FORMSPREE}/**`, (route) => {
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
  test(`a request by ${mode}: the button waits, then our success state`, async ({
    page,
  }) => {
    const value = mode === "email" ? "owner@example.test" : NPUB;
    const requests: Request[] = [];
    let answer!: () => Promise<void>;
    await page.route(`${FORMSPREE}/**`, (route) => {
      requests.push(route.request());
      // Held until the test has seen the button while it's sending.
      answer = () => route.fulfill(accepted);
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
    const sending = beta(page).getByRole("button", { name: "Sending…" });
    await expect(sending).toBeDisabled();
    await expect.poll(() => requests.length).toBe(1);
    await answer();

    const done = beta(page).getByRole("status");
    await expect(done).toContainText("Thanks, you're on the list.");
    await expect(done).toContainText(
      `We'll contact you at ${value} when invites go out.`,
    );
    await expect(
      beta(page).getByText("Thanks, you're on the list."),
    ).toBeFocused();
    await expect(page.locator("#beta-form")).toBeHidden();

    const [request] = requests;
    expect(request.url()).toBe(service.endpoint);
    expect(request.method()).toBe("POST");
    expect(request.headers()["accept"]).toBe("application/json");
    expect(await fields(request)).toEqual({
      [mode]: value,
      contact_type: mode,
      _subject: SUBJECT,
      _gotcha: "",
    });
  });
}

test("Formspree's errors are shown in the form's error area", async ({
  page,
}) => {
  await page.route(`${FORMSPREE}/**`, (route) =>
    route.fulfill({
      status: 422,
      headers: { "Access-Control-Allow-Origin": "*" },
      json: {
        error: "Validation errors",
        errors: [
          { code: "TYPE_EMAIL", field: "email", message: "should be an email" },
          { code: "OTHER", message: "form is not accepting submissions" },
        ],
      },
    }),
  );
  await page.goto("/#beta");
  const input = beta(page).getByLabel("Email address");
  await input.fill("owner@example.test");
  await beta(page).getByRole("button", { name: "Request an invite" }).click();
  const area = page.locator("#beta-error");
  await expect(area).toHaveAttribute("aria-live", "polite");
  await expect(area.getByRole("paragraph")).toHaveText([
    "should be an email",
    "form is not accepting submissions",
  ]);
  await expect(input).toHaveValue("owner@example.test");
  await expect(
    beta(page).getByRole("button", { name: "Request an invite" }),
  ).toBeEnabled();
  await expect(beta(page).getByRole("status")).toBeHidden();
});

for (const failure of ["no connection", "an answer without errors"] as const) {
  test(`with ${failure}, the form says so and offers email`, async ({
    page,
  }) => {
    await page.route(`${FORMSPREE}/**`, (route) =>
      failure === "no connection"
        ? route.abort("connectionrefused")
        : route.fulfill({
            status: 502,
            headers: { "Access-Control-Allow-Origin": "*" },
            contentType: "text/html",
            body: "<h1>Bad gateway</h1>",
          }),
    );
    await page.goto("/#beta");
    const input = beta(page).getByLabel("Email address");
    await input.fill("owner@example.test");
    await beta(page).getByRole("button", { name: "Request an invite" }).click();
    await expect(page.locator("#beta-error")).toHaveText(
      "Couldn't send your request. Try again, or email us at hello@zapbooks.io.",
    );
    await expect(
      beta(page).getByRole("link", { name: "hello@zapbooks.io" }),
    ).toHaveAttribute("href", "mailto:hello@zapbooks.io");
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

  test("the form is an ordinary post to the endpoint", async ({ page }) => {
    let posted: Request | undefined;
    await page.route(`${FORMSPREE}/**`, (route) => {
      posted = route.request();
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
    expect(posted!.method()).toBe("POST");
    expect(await fields(posted!)).toEqual({
      contact_type: "email",
      _subject: SUBJECT,
      _gotcha: "",
      email: "owner@example.test",
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
