import { webcrypto } from "node:crypto";
import { lookup } from "node:dns/promises";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import http from "node:http";
import https from "node:https";
import { isIP } from "node:net";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

interface CipherEnvelope {
  v: number;
  z?: string;
  iter: number;
  salt: string;
  iv: string;
  ct: string;
}

interface CliOpts {
  local: boolean;
  urls: string[];
  plains: string[];
  urlBase: string | null;
}

interface VerifyPair {
  name: string;
  rawBytes?: Buffer;
  url?: string;
  plainPath: string;
  control?: boolean;
}

interface ResolvedAddr {
  address: string;
  family: 4 | 6;
}

const EXPECTED_ITER = 600000;
const EXPECTED_SALT_LEN = 16;
const EXPECTED_IV_LEN = 12;
const MAX_DECOMPRESSED_BYTES = 32 * 1024 * 1024;
const HTTP_TIMEOUT_MS = 20000;

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const DEFAULT_CODE_ROOT =
  SCRIPT_DIR.endsWith("/web/dist") || SCRIPT_DIR.endsWith("\\web\\dist")
    ? resolve(SCRIPT_DIR, "..", "..")
    : SCRIPT_DIR;

function readPackageVersion(): string {
  for (const rel of ["../package.json", "./web/package.json"]) {
    try {
      const parsed = JSON.parse(
        readFileSync(new URL(rel, import.meta.url), "utf8")
      ) as { version?: unknown };
      if (typeof parsed.version === "string" && parsed.version.trim()) {
        return parsed.version.trim();
      }
    } catch {
      // Try next candidate path
    }
  }
  return "0.0.0-dev";
}

const ROOT = process.env.DIGEST_ROOT || DEFAULT_CODE_ROOT;
const VERSION = `ai-digest ${readPackageVersion()}`;
const PASSWORD_FILE =
  process.env.DIGEST_SITE_PASSWORD_FILE ||
  join(homedir(), ".config/ai-digest/site-password");
const PLAIN_ROOT = join(ROOT, "site/plain");
const PUBLIC_ROOT = join(ROOT, "site/public");
const CONFIG = join(ROOT, "site/CONFIG");

const RESERVED_SLUGS = new Set([
  ".",
  "..",
  "_headers",
  "_redirects",
  "_worker.js",
  "_routes.json",
  "robots.txt",
  "404.html",
  "favicon.svg",
  "zh",
  "en",
  "daily",
  "weekly",
  "latest",
]);

const ROOT_LATEST_RELS = [
  "latest.html",
  "daily/latest.html",
  "weekly/latest.html",
  "zh/latest.html",
  "zh/daily/latest.html",
  "zh/weekly/latest.html",
  "en/latest.html",
  "en/daily/latest.html",
  "en/weekly/latest.html",
];

function sanitizeUrlForLog(rawUrl: string): string {
  try {
    const u = new URL(rawUrl);
    u.username = "";
    u.password = "";
    u.search = "";
    u.hash = "";
    return u.toString();
  } catch {
    return rawUrl
      .replace(/^([a-zA-Z][a-zA-Z0-9+.-]*:\/\/)[^/@?#]*@/, "$1")
      .replace(/[?#].*$/, "");
  }
}

function isPublicIpv4(ip: string): boolean {
  const parts = ip.split(".");
  if (parts.length !== 4) return false;
  const octets: number[] = [];
  for (const p of parts) {
    if (!/^\d{1,3}$/.test(p)) return false;
    const n = Number(p);
    if (n < 0 || n > 255) return false;
    octets.push(n);
  }
  const [a, b, c, d] = octets;
  if (a === 0 || a === 10 || a === 127 || a >= 224) return false;
  if (a === 100 && b >= 64 && b <= 127) return false;
  if (a === 169 && b === 254) return false;
  if (a === 172 && b >= 16 && b <= 31) return false;
  if (a === 192 && b === 168) return false;
  if (a === 192 && b === 0 && c === 2) return false;
  if (a === 192 && b === 88 && c === 99) return false;
  if (a === 198 && (b === 18 || b === 19)) return false;
  if (a === 198 && b === 51 && c === 100) return false;
  if (a === 203 && b === 0 && c === 113) return false;
  if (a === 192 && b === 0 && c === 0 && d !== 9 && d !== 10) return false;
  return true;
}

function parseIpv6Segments(rawIp: string): number[] | null {
  let ip = rawIp.trim().toLowerCase();
  if (ip.startsWith("[") && ip.endsWith("]")) ip = ip.slice(1, -1);
  if (!ip || ip.includes("%")) return null;

  let ipv4Tail: number[] | null = null;
  const lastColon = ip.lastIndexOf(":");
  if (lastColon !== -1 && ip.slice(lastColon + 1).includes(".")) {
    const v4 = ip.slice(lastColon + 1);
    const v4Parts = v4.split(".");
    if (v4Parts.length !== 4) return null;
    const octs: number[] = [];
    for (const p of v4Parts) {
      if (!/^\d{1,3}$/.test(p)) return null;
      const n = Number(p);
      if (n < 0 || n > 255) return null;
      octs.push(n);
    }
    ipv4Tail = [(octs[0] << 8) | octs[1], (octs[2] << 8) | octs[3]];
    ip = ip.slice(0, lastColon);
    if (ip.endsWith(":")) ip += "0";
  }

  const halves = ip.split("::");
  if (halves.length > 2) return null;

  const parsePart = (part: string): number[] | null => {
    if (!part) return [];
    const groups = part.split(":");
    const nums: number[] = [];
    for (const g of groups) {
      if (!/^[0-9a-f]{1,4}$/.test(g)) return null;
      nums.push(Number.parseInt(g, 16));
    }
    return nums;
  };

  const left = parsePart(halves[0]);
  const right = halves.length === 2 ? parsePart(halves[1]) : [];
  if (!left || !right) return null;

  const tailLen = ipv4Tail ? 2 : 0;
  if (halves.length === 1) {
    if (left.length + tailLen !== 8) return null;
    return ipv4Tail ? [...left, ...ipv4Tail] : left;
  }

  const missing = 8 - (left.length + right.length + tailLen);
  if (missing < 1) return null;
  const zeros = new Array<number>(missing).fill(0);
  return ipv4Tail
    ? [...left, ...zeros, ...right, ...ipv4Tail]
    : [...left, ...zeros, ...right];
}

function isPublicIpv6(ip: string): boolean {
  const segs = parseIpv6Segments(ip);
  if (!segs) return false;
  const [a, b, c, d, e, f, g, h] = segs;
  // IPv4-mapped IPv6 ::ffff:0:0/96
  if (a === 0 && b === 0 && c === 0 && d === 0 && e === 0 && f === 0xffff) {
    const v4 = `${(g >> 8) & 0xff}.${g & 0xff}.${(h >> 8) & 0xff}.${h & 0xff}`;
    return isPublicIpv4(v4);
  }
  // Global unicast is allocated within 2000::/3 (excludes ::, ::1, fc00::/7, fe80::/10, ff00::/8)
  if (a < 0x2000 || a > 0x3fff) return false;
  if (a === 0x2002 || a === 0x3ffe || a === 0x3fff) return false;
  if (a === 0x2001 && (b < 0x0200 || b === 0x0db8)) return false;
  if (a === 0x2620 && b === 0x004f && c === 0x8000) return false;
  return true;
}

function isPublicIp(ip: string): boolean {
  const kind = isIP(ip);
  if (kind === 4) return isPublicIpv4(ip);
  if (kind === 6) return isPublicIpv6(ip);
  return false;
}

async function validateAndPinTarget(
  rawUrl: string
): Promise<{ parsed: URL; pinned: ResolvedAddr; servername?: string }> {
  const safeUrl = sanitizeUrlForLog(rawUrl);
  let parsed: URL;
  try {
    parsed = new URL(rawUrl);
  } catch {
    throw new Error(`invalid URL: ${safeUrl}`);
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new Error(`unsupported URL scheme (${parsed.protocol}) for ${safeUrl}`);
  }
  if (parsed.username || parsed.password) {
    throw new Error(`URL credentials are forbidden: ${safeUrl}`);
  }
  const rawHost = parsed.hostname;
  const hostNoBrackets =
    rawHost.startsWith("[") && rawHost.endsWith("]")
      ? rawHost.slice(1, -1)
      : rawHost;
  if (!hostNoBrackets) {
    throw new Error(`missing host in URL: ${safeUrl}`);
  }

  const ipKind = isIP(hostNoBrackets);
  if (ipKind === 4 || ipKind === 6) {
    if (!isPublicIp(hostNoBrackets)) {
      throw new Error(`non-public IP target blocked: ${safeUrl}`);
    }
    return {
      parsed,
      pinned: { address: hostNoBrackets, family: ipKind },
    };
  }

  const norm = hostNoBrackets.replace(/\.+$/, "").toLowerCase();
  if (
    !norm ||
    norm === "localhost" ||
    norm === "local" ||
    norm === "internal" ||
    norm.endsWith(".localhost") ||
    norm.endsWith(".local") ||
    norm.endsWith(".internal")
  ) {
    throw new Error(`non-public host blocked: ${safeUrl}`);
  }

  const records = await lookup(norm, { all: true });
  if (!records || records.length === 0) {
    throw new Error(`DNS returned no addresses for ${safeUrl}`);
  }
  for (const rec of records) {
    if (!isPublicIp(rec.address)) {
      throw new Error(
        `host resolved to non-public address (${rec.address}) for ${safeUrl}`
      );
    }
  }
  const first = records[0];
  return {
    parsed,
    pinned: { address: first.address, family: first.family === 6 ? 6 : 4 },
    servername: norm,
  };
}

function loadConfigFlags(): { slug: string; encrypted: boolean } {
  const text = existsSync(CONFIG) ? readFileSync(CONFIG, "utf8") : "";
  let slug =
    process.env.SITE_SLUG !== undefined ? process.env.SITE_SLUG.trim() : "";
  if (process.env.SITE_SLUG === undefined) {
    const m = text.match(/^SLUG=(.*)$/m);
    slug = m ? m[1].trim() : "";
  }
  if (
    slug &&
    (/<[^>]*>|\{\{.*?\}\}|[<>]/.test(slug) ||
      !/^[A-Za-z0-9._~-]+$/.test(slug) ||
      slug.startsWith(".") ||
      RESERVED_SLUGS.has(slug.toLowerCase()))
  ) {
    throw new Error(
      `invalid or unreplaced SLUG (${slug}); run ai-digest publish --init or edit site/CONFIG`
    );
  }
  const envEnc =
    process.env.DIGEST_ENCRYPT !== undefined
      ? process.env.DIGEST_ENCRYPT.trim()
      : "";
  const mEnc = text.match(/^ENCRYPT=(.*)$/m);
  const rawEnc = envEnc || (mEnc ? mEnc[1].trim() : "0");
  const encrypted = /^(1|true|yes|on)$/i.test(rawEnc);
  return { slug, encrypted };
}

function readPassword(): string {
  if (!existsSync(PASSWORD_FILE)) {
    throw new Error(`password file not found: ${PASSWORD_FILE}`);
  }
  let pw = readFileSync(PASSWORD_FILE, "utf8");
  if (pw.endsWith("\n")) pw = pw.slice(0, -1);
  if (pw.endsWith("\r")) pw = pw.slice(0, -1);
  if (pw.length < 16) throw new Error("password file is too short");
  return pw;
}

function extractCfg(html: string): CipherEnvelope {
  const startTag = 'id="c">';
  const i = html.indexOf(startTag);
  if (i < 0) throw new Error("encrypted page is missing the cfg element");
  const j = html.indexOf("</script>", i);
  if (j < 0) throw new Error("encrypted page cfg element is unclosed");
  return JSON.parse(html.slice(i + startTag.length, j)) as CipherEnvelope;
}

function b64ToBytes(s: string): Uint8Array<ArrayBuffer> {
  if (typeof s !== "string" || !/^[A-Za-z0-9+/]*={0,2}$/.test(s)) {
    throw new Error("invalid base64 encoding in cipher envelope");
  }
  const raw = Buffer.from(s, "base64");
  const buf = new ArrayBuffer(raw.byteLength);
  const out = new Uint8Array(buf);
  out.set(raw);
  return out;
}

async function gunzipBounded(
  data: ArrayBuffer,
  maxBytes = MAX_DECOMPRESSED_BYTES
): Promise<Uint8Array<ArrayBuffer>> {
  const stream = new Blob([data])
    .stream()
    .pipeThrough(new DecompressionStream("gzip"));
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!value || value.byteLength === 0) continue;
      total += value.byteLength;
      if (total > maxBytes) {
        await reader.cancel("decompressed payload too large").catch(() => {});
        throw new Error(`decompressed payload exceeds ${maxBytes} bytes`);
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const buf = new ArrayBuffer(total);
  const out = new Uint8Array(buf);
  let offset = 0;
  for (const c of chunks) {
    out.set(c, offset);
    offset += c.byteLength;
  }
  return out;
}

async function derive(
  password: string,
  salt: Uint8Array<ArrayBuffer>,
  iter: number
): Promise<Uint8Array<ArrayBuffer>> {
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

async function decrypt(
  html: string,
  password: string,
  keyCache: Map<string, Uint8Array<ArrayBuffer>>
): Promise<Buffer> {
  const cfg = extractCfg(html);
  if (cfg.v !== 1 && cfg.v !== 2) {
    throw new Error(`unexpected cfg version: ${cfg.v}`);
  }
  if (typeof cfg.iter !== "number" || cfg.iter !== EXPECTED_ITER) {
    throw new Error(
      `unexpected PBKDF2 iterations: ${String(cfg.iter)} (expected ${EXPECTED_ITER})`
    );
  }
  if (cfg.v === 1 && cfg.z !== undefined) {
    throw new Error(`unexpected compression field in v1 envelope: ${String(cfg.z)}`);
  }
  if (cfg.v === 2 && cfg.z !== "gzip") {
    throw new Error(`unexpected compression: ${String(cfg.z)}`);
  }
  const salt = b64ToBytes(cfg.salt);
  if (salt.byteLength !== EXPECTED_SALT_LEN) {
    throw new Error(
      `invalid salt length: ${salt.byteLength} (expected ${EXPECTED_SALT_LEN})`
    );
  }
  const iv = b64ToBytes(cfg.iv);
  if (iv.byteLength !== EXPECTED_IV_LEN) {
    throw new Error(
      `invalid iv length: ${iv.byteLength} (expected ${EXPECTED_IV_LEN})`
    );
  }
  const ct = b64ToBytes(cfg.ct);
  if (ct.byteLength === 0) {
    throw new Error("empty ciphertext in envelope");
  }

  let key = keyCache.get(cfg.salt);
  if (!key) {
    key = await derive(password, salt, EXPECTED_ITER);
    keyCache.set(cfg.salt, key);
  }
  const cryptoKey = await webcrypto.subtle.importKey(
    "raw",
    key,
    "AES-GCM",
    false,
    ["decrypt"]
  );
  const ptBuf = await webcrypto.subtle.decrypt(
    { name: "AES-GCM", iv },
    cryptoKey,
    ct
  );
  if (cfg.v === 2) {
    const out = await gunzipBounded(ptBuf, MAX_DECOMPRESSED_BYTES);
    return Buffer.from(out.buffer, out.byteOffset, out.byteLength);
  }
  if (ptBuf.byteLength > MAX_DECOMPRESSED_BYTES) {
    throw new Error(`plaintext payload exceeds ${MAX_DECOMPRESSED_BYTES} bytes`);
  }
  return Buffer.from(ptBuf);
}

function walkHtmlSync(root: string): string[] {
  const out: string[] = [];
  function rec(dir: string): void {
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
  node web/dist/verify_decrypt.mjs --local
  node web/dist/verify_decrypt.mjs --url-base https://digest.example.com
  node web/dist/verify_decrypt.mjs --url https://digest.example.com/<slug>/daily/2026-01-15.html --plain site/plain/<slug>/daily/2026-01-15.html
`;

function parseArgs(argv: string[]): CliOpts {
  const opts: CliOpts = { local: false, urls: [], plains: [], urlBase: null };
  function needVal(flag: string, i: number): string {
    if (i + 1 >= argv.length) throw new Error(`missing value for ${flag}`);
    return argv[i + 1];
  }
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "-h" || a === "--help") {
      process.stdout.write(HELP);
      process.exit(0);
    } else if (a === "--version") {
      process.stdout.write(`${VERSION}\n`);
      process.exit(0);
    } else if (a === "--local") {
      opts.local = true;
    } else if (a === "--url") {
      opts.urls.push(needVal(a, i));
      i++;
    } else if (a === "--plain") {
      opts.plains.push(needVal(a, i));
      i++;
    } else if (a === "--url-base") {
      opts.urlBase = needVal(a, i);
      i++;
    } else {
      throw new Error(`unknown arg: ${a}`);
    }
  }
  if (opts.urls.length !== opts.plains.length) {
    throw new Error(
      `--url (${opts.urls.length}) and --plain (${opts.plains.length}) must be paired`
    );
  }
  return opts;
}

async function fetchPage(
  rawUrl: string
): Promise<{ status: number; url: string; body: Buffer }> {
  const safeUrl = sanitizeUrlForLog(rawUrl);
  const { parsed, pinned, servername } = await validateAndPinTarget(rawUrl);

  const pinnedLookup = (
    _hostname: string,
    options: unknown,
    cb: (...args: unknown[]) => void
  ): void => {
    if (typeof options === "object" && options !== null && (options as { all?: boolean }).all) {
      cb(null, [{ address: pinned.address, family: pinned.family }]);
    } else {
      cb(null, pinned.address, pinned.family);
    }
  };

  // Dedicated non-proxying direct Agent so NODE_USE_ENV_PROXY / HTTP(S)_PROXY
  // can never bypass the validated pinned IP connection.
  const directAgent =
    parsed.protocol === "https:"
      ? new https.Agent({ keepAlive: false })
      : new http.Agent({ keepAlive: false });

  return new Promise((resolvePromise, rejectPromise) => {
    let settled = false;
    const finishOk = (val: { status: number; url: string; body: Buffer }): void => {
      if (!settled) {
        settled = true;
        directAgent.destroy();
        resolvePromise(val);
      }
    };
    const finishErr = (err: Error): void => {
      if (!settled) {
        settled = true;
        directAgent.destroy();
        rejectPromise(err);
      }
    };
    const mod = parsed.protocol === "https:" ? https : http;
    const req = mod.request(
      parsed,
      {
        method: "GET",
        agent: directAgent,
        timeout: HTTP_TIMEOUT_MS,
        lookup: pinnedLookup as unknown as http.RequestOptions["lookup"],
        ...(servername && parsed.protocol === "https:" ? { servername } : {}),
      },
      (res) => {
        const status = res.statusCode ?? 0;
        if (status !== 200) {
          res.resume();
          finishErr(
            new Error(`${safeUrl} -> HTTP ${status} (expected 200, no redirect)`)
          );
          return;
        }
        const chunks: Buffer[] = [];
        let total = 0;
        res.on("data", (chunk: Buffer) => {
          total += chunk.byteLength;
          if (total > MAX_DECOMPRESSED_BYTES) {
            req.destroy(
              new Error(`${safeUrl} response exceeds ${MAX_DECOMPRESSED_BYTES} bytes`)
            );
            return;
          }
          chunks.push(chunk);
        });
        res.on("end", () => {
          finishOk({
            status,
            url: safeUrl,
            body: Buffer.concat(chunks, total),
          });
        });
        res.on("error", finishErr);
      }
    );
    req.on("timeout", () => {
      req.destroy(new Error(`${safeUrl} request timed out`));
    });
    req.on("error", (err) => {
      finishErr(new Error(`${safeUrl}: ${err.message}`));
    });
    req.end();
  });
}

async function main(): Promise<void> {
  const opts = parseArgs(process.argv.slice(2));
  const { slug, encrypted } = loadConfigFlags();
  const password = encrypted ? readPassword() : null;
  const keyCache = new Map<string, Uint8Array<ArrayBuffer>>();
  const pairs: VerifyPair[] = [];

  const pubBase = slug ? `${PUBLIC_ROOT}/${slug}` : PUBLIC_ROOT;
  const plainBase = slug ? `${PLAIN_ROOT}/${slug}` : PLAIN_ROOT;

  if (opts.local) {
    if (!existsSync(plainBase)) {
      throw new Error(`plaintext directory not found: ${plainBase}`);
    }
    const files = walkHtmlSync(plainBase);
    const nonControlCount = files.filter(
      (f) => f.slice(plainBase.length + 1) !== "404.html"
    ).length;
    if (!files.length || nonControlCount === 0) {
      throw new Error("no HTML pages under site/plain/ (only 404.html or empty)");
    }
    for (const f of files) {
      const rel = f.slice(plainBase.length + 1);
      const pubFile = `${pubBase}/${rel}`;
      if (!existsSync(pubFile)) {
        throw new Error(`published file missing: ${pubFile}`);
      }
      pairs.push({
        name: `local ${rel}`,
        rawBytes: readFileSync(pubFile),
        plainPath: f,
        control: rel === "404.html",
      });
    }
    if (slug) {
      for (const rel of ROOT_LATEST_RELS) {
        const f = `${PUBLIC_ROOT}/${rel}`;
        if (existsSync(f)) {
          pairs.push({
            name: `local /${rel}`,
            rawBytes: readFileSync(f),
            plainPath: `${PLAIN_ROOT}/${rel}`,
            control: false,
          });
        }
      }
    }
  }

  if (opts.urlBase) {
    if (!existsSync(plainBase)) {
      throw new Error(`plaintext directory not found: ${plainBase}`);
    }
    const parsedBase = new URL(opts.urlBase);
    if (parsedBase.search || parsedBase.hash) {
      throw new Error("URL base cannot contain a query or fragment");
    }
    const base = opts.urlBase.replace(/\/$/, "");
    const files = walkHtmlSync(plainBase);
    const liveFiles = files.filter(
      (f) => f.slice(plainBase.length + 1) !== "404.html"
    );
    if (!liveFiles.length) {
      throw new Error("no HTML pages under site/plain/ (only 404.html or empty)");
    }
    for (const f of liveFiles) {
      const rel = f.slice(plainBase.length + 1);
      const relUrl =
        rel === "index.html"
          ? ""
          : rel.endsWith("/index.html")
            ? rel.slice(0, -10)
            : rel.replace(/\.html$/, "");
      const urlPath = slug ? `${slug}/${relUrl}` : relUrl;
      const url = `${base}/${urlPath}`;
      pairs.push({
        name: `GET ${sanitizeUrlForLog(url)}`,
        url,
        plainPath: f,
        control: false,
      });
    }
    if (slug) {
      for (const rel of ROOT_LATEST_RELS) {
        const f = `${PLAIN_ROOT}/${rel}`;
        if (existsSync(f)) {
          const url = `${base}/${rel.replace(/\.html$/, "")}`;
          pairs.push({
            name: `GET ${sanitizeUrlForLog(url)}`,
            url,
            plainPath: f,
            control: false,
          });
        }
      }
    }
  }

  for (let i = 0; i < opts.urls.length; i++) {
    pairs.push({
      name: sanitizeUrlForLog(opts.urls[i]),
      url: opts.urls[i],
      plainPath: opts.plains[i],
      control: false,
    });
  }

  if (!pairs.length) {
    console.error("nothing to verify; pass --local and/or --url-base / --url");
    process.exit(2);
  }

  let failed = 0;
  for (const pair of pairs) {
    try {
      let pubBuf = pair.rawBytes ?? Buffer.alloc(0);
      if (pair.url) {
        const r = await fetchPage(pair.url);
        pubBuf = r.body;
      }
      const bodyText = pubBuf.toString("utf8");
      const expectEncrypted = encrypted && !pair.control;
      let actualBuf = pubBuf;
      if (expectEncrypted) {
        if (!bodyText.includes('id="rm"') || !bodyText.includes('id="pw"')) {
          throw new Error("body is missing the password prompt");
        }
        actualBuf = await decrypt(bodyText, password!, keyCache);
      } else if (bodyText.includes('id="rm"') && bodyText.includes('id="pw"')) {
        throw new Error("public page unexpectedly contains a password prompt");
      }
      if (!pair.plainPath || !existsSync(pair.plainPath)) {
        throw new Error(`plaintext missing: ${pair.plainPath}`);
      }
      const plainBuf = readFileSync(pair.plainPath);
      if (Buffer.compare(actualBuf, plainBuf) !== 0) {
        let pos = 0;
        const n = Math.min(actualBuf.length, plainBuf.length);
        while (pos < n && actualBuf[pos] === plainBuf[pos]) pos++;
        throw new Error(
          `${expectEncrypted ? "decrypted" : "published"} content does not match plaintext ` +
            `(got ${actualBuf.length} bytes, plain ${plainBuf.length} bytes, first diff at ${pos})`
        );
      }
      console.log(`OK  ${pair.name}  (${plainBuf.length} bytes, exact match)`);
    } catch (e) {
      failed += 1;
      const msg = e instanceof Error ? e.message : String(e);
      console.error(`FAIL  ${pair.name}: ${msg}`);
    }
  }
  if (failed) {
    console.error(`${failed} page(s) failed`);
    process.exit(1);
  }
  console.log(
    `all ${pairs.length} page(s) ${encrypted ? "decrypted and " : ""}matched plaintext`
  );
}

main().catch((e) => {
  console.error(e instanceof Error ? e.message : e);
  process.exit(1);
});
