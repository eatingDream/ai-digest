import { chmodSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";

const WEB_DIR = dirname(fileURLToPath(import.meta.url));
const SRC_CSS = join(WEB_DIR, "src", "css");
const SRC_TS = join(WEB_DIR, "src", "ts");
const DIST = join(WEB_DIR, "dist");

interface CssBundleSpec {
  readonly out: string;
  readonly limit: number;
  readonly parts: readonly string[];
}

interface JsEntrySpec {
  readonly entry: string;
  readonly out: string;
}

interface CssBlock {
  head: string;
  body: string;
  raw: boolean;
}

interface CssBuildSize {
  readonly bytes: number;
  readonly limit: number;
}

interface PageBudget {
  readonly name: string;
  readonly bytes: number;
}

interface JsBuildReport {
  readonly sizes: Record<string, number>;
  readonly pageBudgets: readonly PageBudget[];
  readonly jsLimit: number;
}

const CSS_BUNDLES: readonly CssBundleSpec[] = [
  {
    out: "reader.css",
    limit: 22528,
    parts: ["tokens.css", "base.css", "mast.css", "filter.css", "reader.css"],
  },
  {
    out: "index.css",
    limit: 10240,
    parts: ["tokens.css", "base.css", "mast.css", "filter.css", "archive.css"],
  },
  {
    out: "unlock.css",
    limit: 4096,
    parts: ["tokens.css", "base.css", "shell.css", "unlock.css"],
  },
  {
    out: "404.css",
    limit: 4096,
    parts: ["tokens.css", "base.css", "shell.css", "not-found.css"],
  },
];

const JS_IIFE_ENTRIES: readonly JsEntrySpec[] = [
  { entry: "boot.ts", out: "boot.js" },
  { entry: "reader.ts", out: "reader.js" },
  { entry: "index.ts", out: "index.js" },
  { entry: "unlock.ts", out: "unlock.js" },
  { entry: "lock.ts", out: "lock.js" },
];

function normalizeLf(text: string): string {
  return text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trimEnd() + "\n";
}

function byteLen(text: string): number {
  return Buffer.byteLength(text, "utf8");
}

const PRESERVED_CSS_VARS = new Set<string>([
  "--mast-h",
  "--strip-h",
  "--read-prog",
]);

function compactCssVars(css: string): string {
  const varRe = /(?<![A-Za-z0-9_-])--[a-z][a-z0-9-]*/g;
  const counts = new Map<string, number>();
  for (const m of css.matchAll(varRe)) {
    const name = m[0];
    if (!PRESERVED_CSS_VARS.has(name)) {
      counts.set(name, (counts.get(name) ?? 0) + 1);
    }
  }
  const sorted = [...counts.entries()].sort(
    (a, b) => b[1] - a[1] || a[0].localeCompare(b[0]),
  );
  const alpha = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
  const mapping = new Map<string, string>();
  sorted.forEach(([orig], i) => {
    const shortName =
      i < alpha.length
        ? `--${alpha[i]}`
        : `--${alpha[Math.floor(i / alpha.length) - 1]}${alpha[i % alpha.length]}`;
    mapping.set(orig, shortName);
  });
  return css.replace(varRe, (m) => mapping.get(m) ?? m);
}

function mergePrintDisplayNone(body: string): string {
  const hideSelectors: string[] = [];
  const rest: string[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body)) !== null) {
    if (m.index > last) rest.push(body.slice(last, m.index));
    if (m[2] === "display:none!important") {
      hideSelectors.push(m[1] ?? "");
    } else {
      rest.push(m[0]);
    }
    last = re.lastIndex;
  }
  if (last < body.length) rest.push(body.slice(last));
  if (hideSelectors.length <= 1) return body;
  return `${hideSelectors.join(",")}{display:none!important}${rest.join("")}`;
}

function mergeTopLevelBlocks(css: string): string {
  // Parse top-level blocks so we can merge identical :root and trailing @media blocks safely
  const blocks: CssBlock[] = [];
  let i = 0;
  while (i < css.length) {
    const start = i;
    while (i < css.length && css[i] !== "{" && css[i] !== ";") i++;
    if (i >= css.length) {
      const rem = css.slice(start).trim();
      if (rem) blocks.push({ head: "", body: rem, raw: true });
      break;
    }
    if (css[i] === ";") {
      blocks.push({ head: "", body: css.slice(start, i + 1), raw: true });
      i++;
      continue;
    }
    const head = css.slice(start, i).trim();
    let depth = 1;
    const bodyStart = i + 1;
    i++;
    while (i < css.length && depth > 0) {
      if (css[i] === "{") depth++;
      else if (css[i] === "}") depth--;
      i++;
    }
    const body = css.slice(bodyStart, i - 1);
    blocks.push({ head, body, raw: false });
  }

  const mergeFirst = new Set<string>([":root", ":lang(en)", "body"]);
  const mergeLast = new Set<string>([
    "@media(min-width:640px)",
    "@media(min-width:1024px)",
    "@media(prefers-reduced-motion:reduce)",
    "@media(pointer:coarse)",
    "@media print",
  ]);

  const lastIdx = new Map<string, number>();
  blocks.forEach((b, idx) => {
    if (!b.raw && mergeLast.has(b.head)) lastIdx.set(b.head, idx);
  });

  const out: CssBlock[] = [];
  const seenFirst = new Map<string, number>();
  const pendingLast = new Map<string, string>();
  blocks.forEach((b, idx) => {
    if (!b.raw && mergeFirst.has(b.head)) {
      const targetIdx = seenFirst.get(b.head);
      if (targetIdx !== undefined) {
        const target = out[targetIdx];
        if (target) {
          const sep = target.body.endsWith(";") ? "" : ";";
          target.body += sep + b.body;
        }
      } else {
        seenFirst.set(b.head, out.length);
        out.push({ ...b });
      }
    } else if (!b.raw && mergeLast.has(b.head)) {
      const prev = pendingLast.get(b.head) ?? "";
      const combined = prev + b.body;
      if (idx === lastIdx.get(b.head)) {
        // Also merge duplicate :root / :lang(en) inside merged @media blocks
        let inner =
          b.head.startsWith("@media") && combined.includes(":root{")
            ? mergeTopLevelBlocks(combined)
            : combined;
        if (b.head === "@media print") {
          inner = mergePrintDisplayNone(inner);
        }
        out.push({ head: b.head, body: inner, raw: false });
        pendingLast.delete(b.head);
      } else {
        pendingLast.set(b.head, combined);
      }
    } else {
      out.push(b);
    }
  });
  return out
    .map((b) => (b.raw ? b.body : `${b.head}{${b.body}}`))
    .join("");
}

async function buildCss(): Promise<Record<string, CssBuildSize>> {
  const sizes: Record<string, CssBuildSize> = {};
  for (const spec of CSS_BUNDLES) {
    const raw = spec.parts
      .map((name) => readFileSync(join(SRC_CSS, name), "utf8"))
      .join("\n");
    const first = await esbuild.transform(raw, {
      loader: "css",
      target: ["chrome115", "safari16", "firefox115"],
      minify: true,
      legalComments: "none",
      charset: "utf8",
    });
    const merged = mergeTopLevelBlocks(compactCssVars(first.code));
    const second = await esbuild.transform(merged, {
      loader: "css",
      target: ["chrome115", "safari16", "firefox115"],
      minify: true,
      legalComments: "none",
      charset: "utf8",
    });
    const outText = normalizeLf(second.code);
    const bytes = byteLen(outText);
    if (bytes > spec.limit) {
      throw new Error(
        `CSS budget exceeded: ${spec.out} is ${bytes} B (limit ${spec.limit} B)`,
      );
    }
    writeFileSync(join(DIST, spec.out), outText, "utf8");
    sizes[spec.out] = { bytes, limit: spec.limit };
  }
  return sizes;
}

function getOutputText(res: esbuild.BuildResult, label: string): string {
  const file = res.outputFiles?.[0];
  if (!file) {
    throw new Error(`esbuild produced no output for ${label}`);
  }
  return normalizeLf(file.text);
}

function getSize(sizes: Record<string, number>, name: string): number {
  const value = sizes[name];
  if (value === undefined) {
    throw new Error(`Missing built JS size for ${name}`);
  }
  return value;
}

async function buildJs(): Promise<JsBuildReport> {
  const sizes: Record<string, number> = {};

  for (const spec of JS_IIFE_ENTRIES) {
    const res = await esbuild.build({
      entryPoints: [join(SRC_TS, spec.entry)],
      bundle: true,
      format: "iife",
      target: "es2020",
      minify: true,
      legalComments: "none",
      charset: "utf8",
      sourcemap: false,
      write: false,
    });
    const outText = getOutputText(res, spec.entry);
    const bytes = byteLen(outText);
    writeFileSync(join(DIST, spec.out), outText, "utf8");
    sizes[spec.out] = bytes;
  }

  // Cloudflare Pages _worker.js (ESM, preserves literal __WORKER_CFG_JSON__ exactly once)
  {
    const res = await esbuild.build({
      entryPoints: [join(SRC_TS, "worker.ts")],
      bundle: true,
      format: "esm",
      target: "es2020",
      minify: true,
      legalComments: "none",
      charset: "utf8",
      sourcemap: false,
      write: false,
    });
    const outText = getOutputText(res, "worker.ts");
    const placeholderMatches = outText.match(/__WORKER_CFG_JSON__/g) ?? [];
    if (placeholderMatches.length !== 1) {
      throw new Error(
        `worker.js must contain literal __WORKER_CFG_JSON__ exactly once (found ${placeholderMatches.length})`,
      );
    }
    const bytes = byteLen(outText);
    writeFileSync(join(DIST, "worker.js"), outText, "utf8");
    sizes["worker.js"] = bytes;
  }

  // Node >=20 CLI verifier (ESM with shebang)
  {
    const res = await esbuild.build({
      entryPoints: [join(SRC_TS, "verify_decrypt.ts")],
      bundle: true,
      format: "esm",
      platform: "node",
      target: "node20",
      minify: true,
      legalComments: "none",
      charset: "utf8",
      sourcemap: false,
      banner: { js: "#!/usr/bin/env node" },
      write: false,
    });
    const outText = getOutputText(res, "verify_decrypt.ts");
    const bytes = byteLen(outText);
    const outPath = join(DIST, "verify_decrypt.mjs");
    writeFileSync(outPath, outText, "utf8");
    chmodSync(outPath, 0o755);
    sizes["verify_decrypt.mjs"] = bytes;
  }

  // Enforce combined per-page JS budgets (Astra P2: include boot + page + lock / unlock)
  const JS_PAGE_LIMIT = 30720;
  const bootSize = getSize(sizes, "boot.js");
  const lockSize = getSize(sizes, "lock.js");
  const pageBudgets: readonly PageBudget[] = [
    {
      name: "reader (boot + reader + lock)",
      bytes: bootSize + getSize(sizes, "reader.js") + lockSize,
    },
    {
      name: "index (boot + index + lock)",
      bytes: bootSize + getSize(sizes, "index.js") + lockSize,
    },
    {
      name: "unlock (boot + unlock)",
      bytes: bootSize + getSize(sizes, "unlock.js"),
    },
  ];
  for (const pb of pageBudgets) {
    if (pb.bytes > JS_PAGE_LIMIT) {
      throw new Error(
        `JS budget exceeded: ${pb.name} is ${pb.bytes} B (limit ${JS_PAGE_LIMIT} B)`,
      );
    }
  }

  return { sizes, pageBudgets, jsLimit: JS_PAGE_LIMIT };
}

async function main(): Promise<void> {
  mkdirSync(DIST, { recursive: true });
  const cssSizes = await buildCss();
  const { sizes: jsSizes, pageBudgets, jsLimit } = await buildJs();

  console.log("Built web/dist/ artifacts:");
  for (const [name, info] of Object.entries(cssSizes)) {
    console.log(`  CSS  ${name.padEnd(20)} ${String(info.bytes).padStart(6)} / ${info.limit} B`);
  }
  for (const [name, bytes] of Object.entries(jsSizes)) {
    console.log(`  JS   ${name.padEnd(20)} ${String(bytes).padStart(6)} B`);
  }
  console.log("Per-page JS budgets:");
  for (const pb of pageBudgets) {
    console.log(`  ${pb.name.padEnd(32)} ${String(pb.bytes).padStart(6)} / ${jsLimit} B`);
  }
}

main().catch((err: unknown) => {
  console.error(err instanceof Error ? err.message : err);
  process.exit(1);
});
