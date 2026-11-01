// Serves a built folder the way GitHub Pages does, for the site's browser
// tests: files as they are, a folder's index.html, a redirect from a folder
// without its trailing slash, and 404 for anything else. No fallback page,
// so a broken link fails.
//
//   node scripts/serve-static.mjs <dir> <port>

import { createReadStream, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize, resolve, sep } from "node:path";

const [dir = "dist-pages", port = "4180"] = process.argv.slice(2);
const base = resolve(dir);

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".webmanifest": "application/manifest+json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".woff2": "font/woff2",
  ".wasm": "application/wasm",
  ".txt": "text/plain; charset=utf-8",
  ".pdf": "application/pdf",
};

const isFile = (p) => statSync(p, { throwIfNoEntry: false })?.isFile();
const isDir = (p) => statSync(p, { throwIfNoEntry: false })?.isDirectory();

createServer((req, res) => {
  const url = new URL(req.url ?? "/", "http://localhost");
  let path;
  try {
    path = normalize(join(base, decodeURIComponent(url.pathname)));
  } catch {
    path = "";
  }
  if (req.method !== "GET" && req.method !== "HEAD") {
    res.writeHead(405).end();
    return;
  }
  if (path !== base && !path.startsWith(base + sep)) {
    res.writeHead(404).end("Not found");
    return;
  }
  if (isDir(path) && !url.pathname.endsWith("/")) {
    res.writeHead(301, { Location: `${url.pathname}/${url.search}` }).end();
    return;
  }
  const file = isDir(path) ? join(path, "index.html") : path;
  if (!isFile(file)) {
    res.writeHead(404, { "Content-Type": "text/plain" }).end("Not found");
    return;
  }
  res.writeHead(200, {
    "Content-Type": TYPES[extname(file)] ?? "application/octet-stream",
  });
  if (req.method === "HEAD") res.end();
  else createReadStream(file).pipe(res);
}).listen(Number(port), "127.0.0.1", () =>
  console.log(`Serving ${base} at http://127.0.0.1:${port}`),
);
