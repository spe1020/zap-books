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

`npm run test:e2e:site` builds and tests it with an invented form service,
stubbed in the tests.

## One-time setup

### 1. The form service

The beta form posts to a hosted form service. Until its settings are filled
in, `npm run site:build` stops with an error, so the Pages workflow fails
and nothing is deployed.

Edit `site/form-service.json`:

```json
{
  "name": "The service's name, as visitors should see it",
  "endpoint": "https://the form's https address",
  "contactEmail": "the address for the form's 'email us' fallback"
}
```

- `name` appears under the button: "Requests are collected through
  _name_."
- `endpoint` is where the form sends requests. The page's
  Content-Security-Policy allows exactly its origin, for the form's post and
  its script's request, and nothing else.
- `contactEmail` is shown, as a mail link, if a request fails.

The form sends two fields, as an ordinary form post
(`application/x-www-form-urlencoded`):

| Field          | Value                           |
| -------------- | ------------------------------- |
| `contact`      | The email address or the npub   |
| `contact_type` | `email` or `npub`, saying which |

The service needs to:

- accept that post at the endpoint, over https;
- answer a request from the page's script (sent with
  `Accept: application/json`) with a 2xx status when it's saved, and allow
  `https://zapbooks.io` to read the answer (CORS);
- show its own confirmation page when JavaScript is off and the browser
  posts the form itself. The policy lets the form post only to the service's
  origin, so a confirmation page elsewhere, even on zapbooks.io, is blocked.

Turn off any field the service would add by default, so it stores only
those two.

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

Merge to `business-finance-mvp`, or push to it, with the form settings
filled in. The **Pages** workflow builds the demo, assembles the site and
deploys it. Its run shows the address.
