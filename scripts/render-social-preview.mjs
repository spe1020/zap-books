// Renders site/social-preview.png, the 1200×630 image link previews show
// for zapbooks.io: the logo and the headline, in the brand's colors and its
// heading font. Run it again after changing either.
//
//   node scripts/render-social-preview.mjs

import { chromium } from "@playwright/test";
import { readFileSync } from "node:fs";

const file = (path) => readFileSync(new URL(`../${path}`, import.meta.url));
const font = file("public/brand/Outfit-VariableFont_wght.woff2").toString(
  "base64",
);
const logo = file("public/brand/zap-books-logo.svg").toString("base64");

const html = `<!doctype html>
<style>
  @font-face {
    font-family: Outfit;
    src: url(data:font/woff2;base64,${font}) format("woff2");
    font-weight: 100 900;
  }
  html, body { margin: 0; }
  body {
    width: 1200px;
    height: 630px;
    box-sizing: border-box;
    padding: 84px 96px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    background: #fdf9f1;
    border-bottom: 18px solid #ff9f0a;
    font-family: Outfit, sans-serif;
    color: #1b1f27;
  }
  img { height: 76px; width: auto; align-self: flex-start; }
  h1 { margin: 0; font-size: 84px; line-height: 1.04; font-weight: 700; }
  p { margin: 0; font-size: 30px; font-weight: 500; color: #92400e; }
</style>
<img src="data:image/svg+xml;base64,${logo}" alt="">
<h1>Less bookkeeping.<br>More control.</h1>
<p>Your books. Your keys. AI on your terms.</p>`;

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM_PATH || undefined,
});
const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
await page.setContent(html);
await page.evaluate(() => document.fonts.ready);
await page.screenshot({
  path: new URL("../site/social-preview.png", import.meta.url).pathname,
});
await browser.close();
console.log("Wrote site/social-preview.png");
