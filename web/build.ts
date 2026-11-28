import {
  chmodSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";

const WEB_DIR = dirname(fileURLToPath(import.meta.url));
const SRC_CSS = join(WEB_DIR, "src", "css");
const SRC_TS = join(WEB_DIR, "src", "ts");
const DIST = join(WEB_DIR, "dist");
export const UI_THEME_PATH = join(WEB_DIR, "ui-theme.json");

export interface UiThemePalette {
  readonly bg: string;
  readonly bgAlt: string;
  readonly bgMuted: string;
  readonly surface: string;
  readonly surfaceElevated: string;
  readonly well: string;
  readonly mastBg: string;
  readonly ink: string;
  readonly ink2: string;
  readonly ink3: string;
  readonly ink4: string;
  readonly line: string;
  readonly lineStrong: string;
  readonly ctaBg: string;
  readonly ctaInk: string;
  readonly ctaHover: string;
  readonly accent: string;
  readonly accentInk: string;
  readonly accentSoft: string;
  readonly accentSofter: string;
  readonly accentLine: string;
  readonly selection: string;
  readonly hot: string;
  readonly rowHover: string;
  readonly gridLine: string;
  readonly spotGlare: string;
  readonly scrim: string;
}

export interface UiThemeMotion {
  readonly ease: string;
  readonly easeSpring: string;
  readonly pressMs: number;
  readonly fastMs: number;
  readonly baseMs: number;
  readonly mastMs: number;
  readonly indicatorMs: number;
  readonly cardMs: number;
  readonly drawerMs: number;
  readonly drawerCloseDelayMs: number;
  readonly pageTransitionMs: number;
  readonly themeRevealMs: number;
  readonly themeRevealDelayMs: number;
  readonly snapWheelResetMs: number;
  readonly snapWheelLockMs: number;
  readonly sceneNavLockMs: number;
  readonly copyDoneHoldMs: number;
  readonly arrivedHighlightMs: number;
  readonly shortJumpMaxMs: number;
  readonly searchDebounceMs: number;
}

export interface UiThemeConfig {
  readonly name: string;
  readonly palette: {
    readonly dark: UiThemePalette;
    readonly light: UiThemePalette;
  };
  readonly motion: UiThemeMotion;
}

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

export interface CssBuildSize {
  readonly bytes: number;
  readonly limit: number;
  readonly code: string;
}

export interface PageBudget {
  readonly name: string;
  readonly bytes: number;
}

export interface JsBuildReport {
  readonly sizes: Record<string, number>;
  readonly outputs: Record<string, string>;
  readonly pageBudgets: readonly PageBudget[];
  readonly jsLimit: number;
}

export interface BuildOptions {
  readonly theme?: UiThemeConfig;
  readonly write?: boolean;
}

const PALETTE_KEYS: readonly (keyof UiThemePalette)[] = [
  "bg",
  "bgAlt",
  "bgMuted",
  "surface",
  "surfaceElevated",
  "well",
  "mastBg",
  "ink",
  "ink2",
  "ink3",
  "ink4",
  "line",
  "lineStrong",
  "ctaBg",
  "ctaInk",
  "ctaHover",
  "accent",
  "accentInk",
  "accentSoft",
  "accentSofter",
  "accentLine",
  "selection",
  "hot",
  "rowHover",
  "gridLine",
  "spotGlare",
  "scrim",
];

const MOTION_STRING_KEYS: readonly (keyof UiThemeMotion)[] = [
  "ease",
  "easeSpring",
];

const MOTION_MS_KEYS: readonly (keyof UiThemeMotion)[] = [
  "pressMs",
  "fastMs",
  "baseMs",
  "mastMs",
  "indicatorMs",
  "cardMs",
  "drawerMs",
  "drawerCloseDelayMs",
  "pageTransitionMs",
  "themeRevealMs",
  "themeRevealDelayMs",
  "snapWheelResetMs",
  "snapWheelLockMs",
  "sceneNavLockMs",
  "copyDoneHoldMs",
  "arrivedHighlightMs",
  "shortJumpMaxMs",
  "searchDebounceMs",
];

const HEX_COLOR_RE = /^#[0-9A-Fa-f]{6}$/;

function isRecord(val: unknown): val is Record<string, unknown> {
  return typeof val === "object" && val !== null && !Array.isArray(val);
}

function validatePalette(raw: unknown, label: string): UiThemePalette {
  if (!isRecord(raw)) {
    throw new Error(`Invalid ui-theme.json: ${label} must be an object`);
  }
  for (const key of PALETTE_KEYS) {
    const val = raw[key];
    if (typeof val !== "string" || val.trim().length === 0) {
      throw new Error(
        `Invalid ui-theme.json: ${label}.${key} must be a non-empty string`,
      );
    }
    if (/[{};]/.test(val)) {
      throw new Error(
        `Invalid ui-theme.json: ${label}.${key} contains unsafe CSS characters`,
      );
    }
  }
  for (const hexKey of ["bg", "accent"] as const) {
    const v = String(raw[hexKey]);
    if (!HEX_COLOR_RE.test(v)) {
      throw new Error(
        `Invalid ui-theme.json: ${label}.${hexKey} must be a 6-digit hex color (got "${v}")`,
      );
    }
  }
  return raw as unknown as UiThemePalette;
}

export function validateUiTheme(raw: unknown): UiThemeConfig {
  if (!isRecord(raw)) {
    throw new Error("Invalid ui-theme.json: root must be a JSON object");
  }
  if (typeof raw.name !== "string" || raw.name.trim().length === 0) {
    throw new Error("Invalid ui-theme.json: name must be a non-empty string");
  }
  if (!isRecord(raw.palette)) {
    throw new Error("Invalid ui-theme.json: palette must be an object");
  }
  const dark = validatePalette(raw.palette.dark, "palette.dark");
  const light = validatePalette(raw.palette.light, "palette.light");

  if (!isRecord(raw.motion)) {
    throw new Error("Invalid ui-theme.json: motion must be an object");
  }
  for (const key of MOTION_STRING_KEYS) {
    const val = raw.motion[key];
    if (typeof val !== "string" || val.trim().length === 0 || /[{};]/.test(val)) {
      throw new Error(
        `Invalid ui-theme.json: motion.${key} must be a valid CSS timing function`,
      );
    }
  }
  for (const key of MOTION_MS_KEYS) {
    const val = raw.motion[key];
    const min = key === "themeRevealDelayMs" ? 0 : 1;
    if (typeof val !== "number" || !Number.isFinite(val) || val < min) {
      throw new Error(
        `Invalid ui-theme.json: motion.${key} must be a finite number >= ${min}`,
      );
    }
  }

  return {
    name: raw.name,
    palette: { dark, light },
    motion: raw.motion as unknown as UiThemeMotion,
  };
}

export function loadUiTheme(path = UI_THEME_PATH): UiThemeConfig {
  const raw = JSON.parse(readFileSync(path, "utf8")) as unknown;
  return validateUiTheme(raw);
}

function compactColorToken(raw: string): string {
  const trimmed = raw.trim();
  const hex6 = trimmed.match(/^#([0-9a-fA-F]{6})$/);
  if (hex6) {
    const h = hex6[1].toLowerCase();
    if (h[0] === h[1] && h[2] === h[3] && h[4] === h[5]) {
      return `#${h[0]}${h[2]}${h[4]}`;
    }
    return `#${h}`;
  }
  const rgba = trimmed.match(
    /^rgba\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*([0-9]*\.?[0-9]+)\s*\)$/i,
  );
  if (rgba) {
    const r = Math.max(0, Math.min(255, Number(rgba[1])));
    const g = Math.max(0, Math.min(255, Number(rgba[2])));
    const b = Math.max(0, Math.min(255, Number(rgba[3])));
    const a = Math.max(0, Math.min(255, Math.round(Number(rgba[4]) * 255)));
    const hx = [r, g, b, a]
      .map((n) => n.toString(16).padStart(2, "0"))
      .join("");
    if (
      hx[0] === hx[1] &&
      hx[2] === hx[3] &&
      hx[4] === hx[5] &&
      hx[6] === hx[7]
    ) {
      return `#${hx[0]}${hx[2]}${hx[4]}${hx[6]}`;
    }
    return `#${hx}`;
  }
  return trimmed;
}

function compactDurationMs(ms: number): string {
  const asMs = `${ms}ms`;
  const sec = (ms / 1000).toString().replace(/^0\./, ".");
  const asSec = `${sec}s`;
  return asSec.length < asMs.length ? asSec : asMs;
}

function compactTimingFn(fn: string): string {
  return fn
    .trim()
    .replace(/\s*,\s*/g, ",")
    .replace(/\b0+\.(\d)/g, ".$1");
}

export function renderThemeRootCss(theme: UiThemeConfig): string {
  const { dark, light } = theme.palette;
  const { motion } = theme;
  const ld = (key: keyof UiThemePalette): string =>
    `light-dark(${compactColorToken(light[key])},${compactColorToken(dark[key])})`;

  const pairs: readonly [string, string][] = [
    ["--bg", ld("bg")],
    ["--bg-alt", ld("bgAlt")],
    ["--bg-muted", ld("bgMuted")],
    ["--surface", ld("surface")],
    ["--well", ld("well")],
    ["--mast-bg", ld("mastBg")],
    ["--ink", ld("ink")],
    ["--ink-2", ld("ink2")],
    ["--ink-3", ld("ink3")],
    ["--ink-4", ld("ink4")],
    ["--line", ld("line")],
    ["--line-strong", ld("lineStrong")],
    ["--cta-bg", ld("ctaBg")],
    ["--cta-ink", ld("ctaInk")],
    ["--accent", ld("accent")],
    ["--accent-ink", ld("accentInk")],
    ["--accent-soft", ld("accentSoft")],
    ["--accent-softer", ld("accentSofter")],
    ["--selection", ld("selection")],
    ["--hot", ld("hot")],
    ["--scrim", ld("scrim")],
    ["--ease", compactTimingFn(motion.ease)],
    ["--ease-spring", compactTimingFn(motion.easeSpring)],
    ["--dur-press", compactDurationMs(motion.pressMs)],
    ["--dur-fast", compactDurationMs(motion.fastMs)],
    ["--dur-base", compactDurationMs(motion.baseMs)],
    ["--dur-mast", compactDurationMs(motion.mastMs)],
    ["--dur-card", compactDurationMs(motion.cardMs)],
    ["--dur-drawer", compactDurationMs(motion.drawerMs)],
    ["--dur-page", compactDurationMs(motion.pageTransitionMs)],
    ["--dur-arrived", compactDurationMs(motion.arrivedHighlightMs)],
  ];
  return `:root{${pairs.map(([k, v]) => `${k}:${v}`).join(";")}}`;
}

export function buildThemeDefines(theme: UiThemeConfig): Record<string, string> {
  return {
    __THEME_DARK_BG__: JSON.stringify(theme.palette.dark.bg),
    __THEME_LIGHT_BG__: JSON.stringify(theme.palette.light.bg),
    __MOTION_EASE__: JSON.stringify(theme.motion.ease),
    __MOTION_THEME_REVEAL_MS__: String(theme.motion.themeRevealMs),
    __MOTION_THEME_REVEAL_DELAY_MS__: String(theme.motion.themeRevealDelayMs),
    __MOTION_ARRIVED_MS__: String(theme.motion.arrivedHighlightMs),
    __MOTION_SHORT_JUMP_MS__: String(theme.motion.shortJumpMaxMs),
    __MOTION_SEARCH_DEBOUNCE_MS__: String(theme.motion.searchDebounceMs),
    __MOTION_COPY_HOLD_MS__: String(theme.motion.copyDoneHoldMs),
  };
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
    "@media(max-width:960px)",
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

export async function buildCss(
  options: BuildOptions = {},
): Promise<Record<string, CssBuildSize>> {
  const theme = validateUiTheme(options.theme ?? loadUiTheme());
  const write = options.write ?? true;
  const themeRootCss = renderThemeRootCss(theme);
  const sizes: Record<string, CssBuildSize> = {};

  for (const spec of CSS_BUNDLES) {
    const partsRaw = spec.parts
      .map((name) => readFileSync(join(SRC_CSS, name), "utf8"))
      .join("\n");
    const filteredRootCss = themeRootCss.replace(
      /(--[a-z0-9-]+):[^;{}]+;?/g,
      (decl, varName: string) =>
        partsRaw.includes(`var(${varName})`) ? decl : "",
    );
    const raw = [filteredRootCss, partsRaw].join("\n");
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
    if (write) {
      writeFileSync(join(DIST, spec.out), outText, "utf8");
    }
    sizes[spec.out] = { bytes, limit: spec.limit, code: outText };
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

export async function buildJs(options: BuildOptions = {}): Promise<JsBuildReport> {
  const theme = validateUiTheme(options.theme ?? loadUiTheme());
  const write = options.write ?? true;
  const defines = buildThemeDefines(theme);
  const sizes: Record<string, number> = {};
  const outputs: Record<string, string> = {};

  for (const spec of JS_IIFE_ENTRIES) {
    const res = await esbuild.build({
      entryPoints: [join(SRC_TS, spec.entry)],
      bundle: true,
      format: "iife",
      target: "es2020",
      define: defines,
      minify: true,
      legalComments: "none",
      charset: "utf8",
      sourcemap: false,
      write: false,
    });
    const outText = getOutputText(res, spec.entry);
    const bytes = byteLen(outText);
    if (write) {
      writeFileSync(join(DIST, spec.out), outText, "utf8");
    }
    sizes[spec.out] = bytes;
    outputs[spec.out] = outText;
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
    if (write) {
      writeFileSync(join(DIST, "worker.js"), outText, "utf8");
    }
    sizes["worker.js"] = bytes;
    outputs["worker.js"] = outText;
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
    if (write) {
      const outPath = join(DIST, "verify_decrypt.mjs");
      writeFileSync(outPath, outText, "utf8");
      chmodSync(outPath, 0o755);
    }
    sizes["verify_decrypt.mjs"] = bytes;
    outputs["verify_decrypt.mjs"] = outText;
  }

  // Enforce combined per-page JS budgets (include boot + page + lock / unlock)
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

  return { sizes, outputs, pageBudgets, jsLimit: JS_PAGE_LIMIT };
}

async function main(): Promise<void> {
  mkdirSync(DIST, { recursive: true });
  const theme = loadUiTheme();
  const cssSizes = await buildCss({ theme, write: true });
  const { sizes: jsSizes, pageBudgets, jsLimit } = await buildJs({
    theme,
    write: true,
  });

  console.log(`Built web/dist/ artifacts (${theme.name}):`);
  for (const [name, info] of Object.entries(cssSizes)) {
    console.log(
      `  CSS  ${name.padEnd(20)} ${String(info.bytes).padStart(6)} / ${info.limit} B`,
    );
  }
  for (const [name, bytes] of Object.entries(jsSizes)) {
    console.log(`  JS   ${name.padEnd(20)} ${String(bytes).padStart(6)} B`);
  }
  console.log("Per-page JS budgets:");
  for (const pb of pageBudgets) {
    console.log(
      `  ${pb.name.padEnd(32)} ${String(pb.bytes).padStart(6)} / ${jsLimit} B`,
    );
  }
}

const isMain =
  process.argv[1] !== undefined &&
  realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));

if (isMain) {
  main().catch((err: unknown) => {
    console.error(err instanceof Error ? err.message : err);
    process.exit(1);
  });
}
