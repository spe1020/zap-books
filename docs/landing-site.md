# The zapbooks.io site

zapbooks.io is the landing page in `site/` and, at `/demo/`, a demo-only
build of the app. `.github/workflows/pages.yml` builds both and deploys them
to GitHub Pages on every push to `business-finance-mvp`. Pull requests never
deploy.

## What's where

- `site/`: the page (`index.html`, `styles.css`), the beta form's script
  (`beta-form.js`), the manifest, the social preview image and `CNAME`.
  Plain files, no framework.
- `site/form-service.json`: the beta form's settings (below).
- `scripts/build-pages.mjs` (`npm run site:build`): copies `site/` and the
  brand files it uses into `dist-pages/`, fills in the form settings, and
  adds the demo from `dist-demo/` at `/demo/`.
- `npm run build:demo`: the demo-only build (`VITE_DEMO_ONLY=true`, served
  from `/demo/`). It opens the local demo directly, with no sign-in and no
  receipt reading, and its Content-Security-Policy allows requests only to
  its own origin.
- `scripts/render-social-preview.mjs`: redraws `site/social-preview.png`
  after the logo or the headline changes.

To look at it on your computer:

```bash
npm run build:demo
npm run site:build
npm run site:serve    # http://127.0.0.1:4180
```

`npm run test:e2e:site` builds and tests it, with Formspree stubbed.

## One-time setup

### 1. The form service: Formspree

The beta form posts to Formspree. Its settings are in
`site/form-service.json`:

```json
{
  "name": "Formspree",
  "endpoint": "https://formspree.io/f/xdeewqvn",
  "contactEmail": "hello@zapbooks.io"
}
```

- `name` appears under the button: "Requests are collected through
  Formspree."
- `endpoint` is the form's `action`, and where its script sends requests.
  The page's Content-Security-Policy allows exactly its origin,
  `https://formspree.io`, in `form-action` and `connect-src`, and nothing
  else.
- `contactEmail` is shown, as a mail link, if a request can't be sent.

`npm run site:build` refuses a placeholder or a missing value, so the
Pages workflow never deploys a form that can't work.

The form sends these fields, and nothing else:

| Field               | Value                                                                        |
| ------------------- | ---------------------------------------------------------------------------- |
| `email` _or_ `npub` | The email address or the Nostr public key, whichever was used                |
| `contact_type`      | `email` or `npub`, saying which                                              |
| `_subject`          | `Zap Books beta request`, the subject of Formspree's email                   |
| `_gotcha`           | Empty. A honeypot nobody can see or reach; a bot that fills it in is dropped |

Without JavaScript the browser posts the form itself, and Formspree shows
its own confirmation page. With JavaScript the page checks the address,
then sends the same fields with `fetch` and `Accept: application/json`:

- `200` with `{ "ok": true }`: the page's own success message;
- an answer with `errors`: each error's `message`, in the form's error
  area;
- no answer, or any other answer: "Couldn't send your request", with a link
  to email `hello@zapbooks.io`.

The browser tests (`npm run test:e2e:site`) build the site with these
settings and stub every request to Formspree, so none is sent.

In Formspree, check the form's settings: it should accept AJAX requests
from `https://zapbooks.io` (its allowed domains, if you restrict them).

### 2. Turn on Pages

In the repository: **Settings → Pages → Build and deployment → Source:
GitHub Actions**.

The workflow deploys to the `github-pages` environment, which GitHub creates
the first time. By default it accepts deployments only from the default
branch, which is `business-finance-mvp`.

### 3. Verify the domain

Verifying zapbooks.io stops anyone else's Pages site from claiming it, or
its subdomains.

1. On GitHub: **your profile picture → Settings → Pages → Add a domain**,
   and enter `zapbooks.io`.
2. GitHub shows a TXT record, named `_github-pages-challenge-<your
username>.zapbooks.io`. Add it at your DNS provider with the value
   GitHub shows.
3. Back on GitHub, choose **Verify**. DNS can take a while to update; try
   again later if it isn't found at first.

### 4. DNS records

At your DNS provider, for `zapbooks.io`. These are GitHub's published
addresses for Pages; check
[GitHub's documentation](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site/managing-a-custom-domain-for-your-github-pages-site)
for any change before adding them.

| Name  | Type  | Value                 |
| ----- | ----- | --------------------- |
| `@`   | A     | `185.199.108.153`     |
| `@`   | A     | `185.199.109.153`     |
| `@`   | A     | `185.199.110.153`     |
| `@`   | A     | `185.199.111.153`     |
| `@`   | AAAA  | `2606:50c0:8000::153` |
| `@`   | AAAA  | `2606:50c0:8001::153` |
| `@`   | AAAA  | `2606:50c0:8002::153` |
| `@`   | AAAA  | `2606:50c0:8003::153` |
| `www` | CNAME | `spe1020.github.io`   |

Remove any other A, AAAA or CNAME records for those names first. GitHub
redirects `www.zapbooks.io` to `zapbooks.io`.

### 5. The custom domain and HTTPS

1. **Settings → Pages → Custom domain:** enter `zapbooks.io` and save.
   With GitHub Actions as the source, this setting is what counts; the
   `site/CNAME` file is kept so the folder says where it's served.
2. GitHub checks the DNS records and then issues a certificate. That can
   take up to a day.
3. Once it's issued, tick **Enforce HTTPS**.

### 6. Deploy

Merge to `business-finance-mvp`, or push to it. The **Pages** workflow builds the demo, assembles the site and
deploys it. Its run shows the address.
