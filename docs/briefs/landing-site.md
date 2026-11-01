# Brief: The zapbooks.io landing page and hosted demo

**For:** Claude Code
**Repo:** `spe1020/zap-books`
**Branch:** `site/landing`
**Save this brief as:** `docs/briefs/landing-site.md` (Prettier only)
**Design source:** the attached `landing-design.dc.html`. It's an export from a design canvas, in a templated format. Use it for **content, structure, copy and styling**; don't ship the file itself.

**Privacy rule:** invented values only. The guard must pass on every commit and PR description.

## Decisions already made

- **Hosting:** GitHub Pages, from this repo, at `https://zapbooks.io`.
- **License: MIT.**
- **Beta form:** a hosted form service. Seth will create the form and give you its endpoint, so make it a single configuration value.
- **Pricing:** simplified (see Content).

## Commit 1: license

- Add `LICENSE` (MIT, `Copyright (c) 2026 Zap Cooking LLC`; Seth will change the holder if needed).
- Set `license = "MIT"` in the Cargo workspace and `"license": "MIT"` in `package.json`.
- Add a License section to the README.
- Confirm no dependency's license conflicts with MIT distribution, and list anything notable in the PR.

## Commit 2: the static site in `site/`

- **Plain HTML and CSS,** with a small amount of vanilla JavaScript only for the form. No framework, no build step beyond copying files.
- **Brand:** the logo and icons from `public/brand/`, Outfit for headings (the woff2 already in `public/brand/`), and the warm palette. Self-host every font; **no requests to any third-party host except the form submission.**
- **A Content-Security-Policy** in a `<meta>` tag: `default-src 'self'`, with only the form service's origin allowed in `form-action` and `connect-src`. No inline scripts.
- Favicons, the web manifest, a social preview image (1200×630, built from the logo and headline), and meta tags for the description and Open Graph. Add a `CNAME` file containing `zapbooks.io`.
- **It must work without JavaScript:** the form falls back to an ordinary form post.
- **Accessibility:** WCAG AA contrast, visible focus, real buttons and links, `<details>` for the expandable panels, and correct heading order.
- **Phone first:** nothing overflows at 360 px wide.

### Content (from the design, with these changes)

- **Keep:**
  - the hero;
  - the Review-screen preview (labeled sample data);
  - the three steps;
  - "What you can do today", including the beta limitations;
  - the bitcoin roadmap section;
  - the privacy, ownership and verification section with both expandable panels;
  - the FAQ;
  - the footer.
- **Update "What you can do today" for bitcoin, which now exists:** add "Keep books in sats", covering wallet accounts, sats entry, the dollar value recorded at the time of each bitcoin entry, buying and selling bitcoin, and bitcoin exports. Change the roadmap's **Books in sats** card to "Cost basis and gains", noting that books in sats are available now. Remove "no bitcoin" from the beta limitations, and update the FAQ answer: you can keep books in sats today, and cost basis and gains aren't tracked yet.
- **Open source:** the code is now MIT-licensed. "Open source" may be stated, and **Self-hosted, free** becomes a real option, linking to the repository and its setup docs.
- **Pricing, simplified:**
  - The founding beta banner: "Free during the beta. Then $79 a year for the first 50 businesses."
  - Below it: "Solo from $99 a year · Business from $199 a year", with "Full plan details before billing starts."
  - The AI note (the receipt reader runs on your computer; the assistant is planned).
  - Remove every "[to confirm]" placeholder and the per-plan limit lists.
- **The beta form:**
  - email by default, with a "Use a Nostr public key instead" switch;
  - the same validation and the accessible success and error states as the design;
  - **real submission** to the form service, sending only the email or npub plus a hidden field saying which;
  - a clear error if the submission fails, with a fallback "email us" link;
  - under the button, name the form service: "Requests are collected through [service]. We only use this to send your invite."
- **"Try the demo"** goes to `/demo/` (Commit 3).
- No testimonials, invented numbers, or claims beyond what the code does. When in doubt, cut it.

## Commit 3: the hosted demo at `/demo/`

- **A demo-only build** of the app, served from `/demo/`, with **no service configured**. It opens the local demo directly: no sign-in screen, no remote mode, no extractor prompt that can't work. If the app can't do this without code changes, add a build flag (for example `VITE_DEMO_ONLY=true`) that hides remote mode, with a test.
- Show a slim banner: "Demo with fictional data. Everything stays in your browser." with a link back to `/`.
- Asset paths work under `/demo/` (Vite `base`).
- The demo's own CSP stays strict and allows WebAssembly, as the app's does.

## Commit 4: deployment workflow

- `.github/workflows/pages.yml`: on pushes to `business-finance-mvp`, it builds the demo, assembles `site/` plus the demo into one folder, and deploys with the official Pages actions, **pinned to commit hashes** like the rest of CI.
- **Permissions:** only this workflow gets `pages: write` and `id-token: write`. The main CI workflow stays read-only.
- Don't deploy from pull requests.
- `docs/landing-site.md` gives Seth the one-time steps:
  - enable Pages with "GitHub Actions" as the source;
  - the DNS records for `zapbooks.io` and `www` at his DNS provider (GitHub's apex A/AAAA records and the `www` CNAME);
  - verify the domain in GitHub;
  - turn on "Enforce HTTPS";
  - where to paste the form endpoint.

## Checks

- All existing checks still pass.
- A Playwright test for the site at 360 px and 1280 px:
  - every link and anchor resolves;
  - no horizontal overflow;
  - accessibility scans pass;
  - form validation for both modes;
  - success and failure states, with the form service stubbed;
  - the no-JavaScript fallback.
- A Playwright test that `/demo/` opens the demo with no sign-in and no failed requests.
- Screenshots of the site and the demo, at both widths, in the PR.

## Stop and ask if

- the app can't run demo-only without changes beyond a build flag;
- the CSP can't stay strict for the form service;
- any copy would need a claim you can't verify in the code.

Open the PR when the checks pass, and stop.
