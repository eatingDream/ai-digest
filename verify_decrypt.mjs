#!/usr/bin/env node
/**
 * Headless decrypt check: AES-256-GCM + PBKDF2-SHA256 via WebCrypto,
 * matching the in-page unlock script. Reads the password from
 * DIGEST_SITE_PASSWORD_FILE (default ~/.config/ai-digest/site-password).
 * Never prints the password.
 *
 * Usage:
 *   node verify_decrypt.mjs --local
 *   node verify_decrypt.mjs --url-base https://digest.example.com
 *   node verify_decrypt.mjs --url URL --plain PATH
 */
import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { webcrypto } from "node:crypto";

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";

const ROOT = process.env.DIGEST_ROOT || dirname(fileURLToPath(import.meta.url));
const VERSION = (() => {
  try {
    const m = readFileSync(join(ROOT, "version.py"), "utf8").match(/__version__\s*=\s*"([^"]+)"/);
    return `ai-digest ${m ? m[1] : "unknown"}`;
  } catch {
    return "ai-digest unknown";
  }
})();
const PASSWORD_FILE =
  process.env.DIGEST_SITE_PASSWORD_FILE ||
  join(homedir(), ".config/ai-digest/site-password");
const PLAIN_ROOT = join(ROOT, "site/plain");
const PUBLIC_ROOT = join(ROOT, "site/public");
const CONFIG = join(ROOT, "site/CONFIG");

function loadSlug() {
  if (process.env.SITE_SLUG) return process.env.SITE_SLUG.trim();
  const text = readFileSync(CONFIG, "utf8");
  const m = text.match(/^SLUG=(.+)$/m);
  if (!m) throw new Error("SLUG missing from CONFIG");
  return m[1].trim();
}

function readPassword() {
  if (!existsSync(PASSWORD_FILE)) {
    throw new Error(`password file not found: ${PASSWORD_FILE}`);
  }
  let pw = readFileSync(PASSWORD_FILE, "utf8");
  if (pw.endsWith("\n")) pw = pw.slice(0, -1);
  if (pw.endsWith("\r")) pw = pw.slice(0, -1);
  if (pw.length < 16) throw new Error("password file is too short");
  return pw;
}

function extractCfg(html) {
  const startTag = 'id="c">';
  const i = html.indexOf(startTag);
  if (i < 0) throw new Error("encrypted page is missing the cfg element");
  const j = html.indexOf("</script>", i);
  if (j < 0) throw new Error("encrypted page cfg element is unclosed");
  return JSON.parse(html.slice(i + startTag.length, j));
}

function b64ToBytes(s) {
  return Uint8Array.from(Buffer.from(s, "base64"));
}

async function derive(password, salt, iter) {
  const base = await webcrypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(password),
    "PBKDF2",
    false,
    ["deriveBits"]
  );
  const bits = await webcrypto.subtle.deriveBits(
    { name: "PBKDF2", salt, iterations: iter, hash: "SHA-256" },
    base,
    256
  );
  return new Uint8Array(bits);
}

async function decrypt(html, password, keyCache) {
  const CFG = extractCfg(html);
  if (CFG.v !== 1 && CFG.v !== 2) throw new Error(`unexpected cfg version: ${CFG.v}`);
  if (CFG.iter < 600000) throw new Error(`iterations too low: ${CFG.iter}`);
  const salt = b64ToBytes(CFG.salt);
  let key = keyCache.get(CFG.salt);
  if (!key) {
    key = await derive(password, salt, CFG.iter);
    keyCache.set(CFG.salt, key);
  }
  const cryptoKey = await webcrypto.subtle.importKey(
    "raw",
    key,
    "AES-GCM",
    false,
    ["decrypt"]
  );
  let pt = await webcrypto.subtle.decrypt(
    { name: "AES-GCM", iv: b64ToBytes(CFG.iv) },
    cryptoKey,
    b64ToBytes(CFG.ct)
  );
  if (CFG.v === 2) {
    if (CFG.z !== "gzip") throw new Error(`unexpected compression: ${CFG.z}`);
    const s = new Blob([pt]).stream().pipeThrough(new DecompressionStream("gzip"));
    pt = await new Response(s).arrayBuffer();
  }
  return new TextDecoder("utf-8").decode(pt);
}

function walkHtmlSync(root) {
  const out = [];
  function rec(dir) {
    for (const name of readdirSync(dir)) {
      const p = `${dir}/${name}`;
      const st = statSync(p);
      if (st.isDirectory()) rec(p);
      else if (name.endsWith(".html")) out.push(p);
    }
  }
  rec(root);
  return out.sort();
}

const HELP = `usage: verify_decrypt.mjs [-h] [--version] [--local] [--url-base URL] [--url URL --plain PATH]

Headless decrypt check for the published pages (AES-256-GCM + PBKDF2-SHA256 via WebCrypto).
Reads the password from DIGEST_SITE_PASSWORD_FILE (default ~/.config/ai-digest/site-password);
never prints it. Exit code 0 = every checked page decrypted; non-zero = a failure.

options:
  -h, --help       show this help and exit
  --version        print the version and exit
  --local          decrypt every page under site/public/ and compare with site/plain/
  --url-base URL   fetch and decrypt the live pages under URL (e.g. https://digest.example.com)
  --url URL        decrypt one page at URL (repeatable; pair with --plain)
  --plain PATH     plaintext file to compare with the matching --url (repeatable)

examples:
  node verify_decrypt.mjs --local
  node verify_decrypt.mjs --url-base https://digest.example.com
  node verify_decrypt.mjs --url https://digest.example.com/<slug>/daily/2026-01-15.html --plain site/plain/<slug>/daily/2026-01-15.html
`;

function parseArgs(argv) {
  const opts = { local: false, urls: [], plains: [], urlBase: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "-h" || a === "--help") { process.stdout.write(HELP); process.exit(0); }
    else if (a === "--version") { process.stdout.write(`${VERSION}\n`); process.exit(0); }
    else if (a === "--local") opts.local = true;
    else if (a === "--url") opts.urls.push(argv[++i]);
    else if (a === "--plain") opts.plains.push(argv[++i]);
    else if (a === "--url-base") opts.urlBase = argv[++i];
    else throw new Error(`unknown arg: ${a}`);
  }
  return opts;
}

async function fetchText(url) {
  const res = await fetch(url, { redirect: "manual" });
  if (res.status !== 200) throw new Error(`${url} -> HTTP ${res.status} (expected 200, no redirect)`);
  return { status: res.status, url: res.url, body: await res.text() };
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const password = readPassword();
  const keyCache = new Map();
  const slug = loadSlug();
  const pairs = [];

  if (opts.local) {
    const pub = `${PUBLIC_ROOT}/${slug}`;
    const plain = `${PLAIN_ROOT}/${slug}`;
    const files = walkHtmlSync(pub);
    if (!files.length) throw new Error("no encrypted HTML under public/");
    for (const f of files) {
      const rel = f.slice(pub.length + 1);
      pairs.push({
        name: `local ${rel}`,
        enc: readFileSync(f, "utf8"),
        plainPath: `${plain}/${rel}`,
      });
    }
  }

  if (opts.urlBase) {
    const plain = `${PLAIN_ROOT}/${slug}`;
    const files = walkHtmlSync(plain);
    for (const f of files) {
      const rel = f.slice(plain.length + 1);
      // Extensionless, exactly as the site links them (no 308 hop).
      const urlPath = rel === "index.html" ? `${slug}/` : `${slug}/${rel.replace(/\.html$/, "")}`;
      const url = `${opts.urlBase.replace(/\/$/, "")}/${urlPath}`;
      pairs.push({ name: `GET ${url}`, url, plainPath: f });
    }
  }

  for (let i = 0; i < opts.urls.length; i++) {
    pairs.push({
      name: opts.urls[i],
      url: opts.urls[i],
      plainPath: opts.plains[i],
    });
  }

  if (!pairs.length) {
    console.error("nothing to verify; pass --local and/or --url-base / --url");
    process.exit(2);
  }

  let failed = 0;
  for (const pair of pairs) {
    try {
      let enc = pair.enc;
      if (pair.url) {
        const r = await fetchText(pair.url);
        enc = r.body;
      }
      if (!enc.includes('id="rm"') || !enc.includes('id="pw"')) {
        throw new Error("body is missing the password prompt");
      }
      const dec = await decrypt(enc, password, keyCache);
      if (!pair.plainPath || !existsSync(pair.plainPath)) {
        throw new Error(`plaintext missing: ${pair.plainPath}`);
      }
      const expected = readFileSync(pair.plainPath, "utf8");
      if (dec !== expected) {
        const a = Buffer.from(dec);
        const b = Buffer.from(expected);
        let pos = 0;
        const n = Math.min(a.length, b.length);
        while (pos < n && a[pos] === b[pos]) pos++;
        throw new Error(
          `decrypted content does not match plaintext ` +
            `(dec ${a.length} bytes, plain ${b.length} bytes, first diff at ${pos})`
        );
      }
      console.log(`OK  ${pair.name}  (${expected.length} bytes, exact match)`);
    } catch (e) {
      failed += 1;
      console.error(`FAIL  ${pair.name}: ${e.message}`);
    }
  }
  if (failed) {
    console.error(`${failed} page(s) failed`);
    process.exit(1);
  }
  console.log(`all ${pairs.length} page(s) decrypted and matched plaintext`);
}

main().catch((e) => {
  console.error(e.message || e);
  process.exit(1);
});
