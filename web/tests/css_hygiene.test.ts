import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  buildCss,
  buildJs,
  loadUiTheme,
  renderThemeRootCss,
  UI_THEME_PATH,
  type UiThemeConfig,
} from "../build.ts";

const TESTS_DIR = dirname(fileURLToPath(import.meta.url));
const DIST = join(TESTS_DIR, "..", "dist");

const EXPECTED_DIST_FILES = [
  "reader.css",
  "index.css",
  "unlock.css",
  "404.css",
  "boot.js",
  "reader.js",
  "index.js",
  "unlock.js",
  "lock.js",
  "worker.js",
  "verify_decrypt.mjs",
];

// Representative fixture HTML covering all conditional states for each page type
const READER_FIXTURE_HTML = `<!DOCTYPE html>
<html lang="zh-CN">
<body>
<a class="skip-link" href="#main">Skip</a>
<div class="mast-sentinel"></div>
<header class="mast">
  <div class="mast-row">
    <span class="np"><a class="np-link" href="/" data-brand-home><span class="ai">AI</span> Digest</a></span>
    <span class="edition" data-kind="daily">Daily</span>
    <span class="mast-sep" aria-hidden="true"></span>
    <nav class="issue-nav top"><a href="/prev" rel="prev"><span data-short-date>11.25</span></a><a class="nav-index" href="/"><span class="nav-lbl">Archive</span></a><a class="nav-week" href="/weekly/latest">Weekly</a></nav>
    <div class="mast-tools">
      <button type="button" class="ctrl-btn search-btn" data-filter-toggle aria-expanded="false" aria-controls="digestFilterBar"><span>Search</span><kbd>/</kbd></button>
      <div class="font-seg" role="group"><button type="button" class="font-btn" data-font-size="sm" aria-pressed="false">S</button><button type="button" class="font-btn" data-font-size="md" aria-pressed="true">M</button><button type="button" class="font-btn" data-font-size="lg" aria-pressed="false">L</button></div>
      <div class="lang-seg"><span aria-current="page">ZH</span><a class="ctrl-btn lang-btn" data-lang-link href="/en/">EN</a></div>
      <button type="button" class="ctrl-btn theme-btn" data-theme-toggle></button>
      <button type="button" class="ctrl-btn lock-btn" data-lock></button>
    </div>
  </div>
</header>
<div class="read-prog"><div class="read-prog-fill"></div></div>
<div class="search-scrim" data-search-scrim hidden></div>
<div class="filter-bar filter-drawer search-panel" id="digestFilterBar" hidden aria-hidden="true" inert>
  <div class="filter-inner">
    <div class="filter-input-wrap">
      <input type="search" class="filter-input">
      <span class="filter-count">1</span>
      <button type="button" class="filter-clear" hidden>Clear</button>
      <button type="button" class="filter-close">Close</button>
    </div>
    <div class="filter-tags">
      <button type="button" class="filter-tag" data-tag="llm">LLM<span class="tag-cnt num">1</span></button>
      <button type="button" class="filter-tag filter-more" hidden>+1 more</button>
    </div>
  </div>
</div>
<div class="reader-shell">
  <nav class="toc-side" id="toc-sheet">
    <div class="toc-top">
      <div class="toc-handle"></div>
      <div class="toc-head"><h2 class="toc-label">Contents</h2><button type="button" class="toc-expand-all">All</button><button type="button" class="toc-close">X</button></div>
      <div class="cat-chips"><button type="button" class="chip" data-cat="all" aria-pressed="true">All <b class="n num">1</b></button></div>
    </div>
    <div class="toc-front"><a href="#front-must">Must</a></div>
    <div class="ts-group is-open" data-sec="sec-1">
      <button type="button" class="ts-sec" aria-expanded="true" aria-controls="ts-list-sec-1">
        <span class="ts-no num">01</span><span class="ts-title">Models</span><span class="ts-cnt num" data-total="1">1</span>
      </button>
      <div class="ts-panel" id="ts-list-sec-1"><ol class="ts-items"><li><a href="#item-1" title="Title"><span class="ts-num num">1.</span><span class="ts-title short">Title</span><span class="dot-hot"></span></a></li></ol></div>
    </div>
  </nav>
  <main id="main" class="prose-col">
    <header class="folio issue-head">
      <h1 class="issue-title"><time datetime="2026-11-26">Nov 26</time><span class="vh"> AI Digest Daily</span></h1>
      <p class="folio-dek">Summary</p>
      <p class="issue-meta mast-meta"><span data-k="read"><b>3</b> min</span><span data-k="count"><b>1</b> item</span></p>
      <div class="mobile-filter-strip"><button type="button" class="chip" data-cat="all" aria-pressed="true">All <b class="n num">1</b></button></div>
      <nav class="folio-sections"><a href="#sec-1">01 Models</a></nav>
    </header>
    <section class="issue-summary"><h2 id="sum-h">Summary</h2><p class="summary-lead">Lead <a href="#item-1">Story</a>.</p></section>
    <div class="filter-banner" data-filter-banner hidden><span class="fb-text" data-fb-text></span><button type="button" class="filter-clear fb-clear">Clear filter</button></div>
    <section class="front front-must" id="front-must">
      <h2>Must Read</h2>
      <ol class="must-list"><li><span class="must-n num">01</span><div class="must-body"><a class="must-title" href="#item-1">Lead</a><p class="must-dek">Dek</p></div></li></ol>
    </section>
    <section class="front front-lead" id="front-themes">
      <div class="themes"><div class="theme-item"><h4>Theme</h4><p>Text <a href="#item-1">#1</a></p></div></div>
    </section>
    <section class="sec" id="sec-1">
      <div class="sec-head"><span class="sec-no num">01</span><h2>Models</h2><span class="sec-cnt num">1</span><p class="sec-note">Note</p></div>
      <h3 class="subhead">Subhead</h3>
      <article class="story story--lead" id="item-1" data-tags="llm">
        <div class="story-meta">
          <span class="source">Source</span>
          <button type="button" class="badge badge-accent" data-tag="llm">LLM</button>
          <span class="badge badge-hot">New</span>
          <time class="when">11-26</time>
          <button type="button" class="copy-link" data-copy-anchor="item-1"></button>
        </div>
        <div class="story-head"><h3><span class="story-no">1.</span> Story</h3></div>
        <p class="dek">Dek</p>
        <aside class="look"><span class="look-k">Why</span><div class="prose"><p><span class="math math-wrap"><span class="frac">1/2</span><span class="sqrt">x</span></span> <code class="math-raw">$raw$</code></p></div></aside>
        <div class="detail"><span class="detail-k">Details</span><div class="prose"><p>Detail</p></div></div>
        <div class="extra"><span class="extra-k">More</span><div class="prose"><p>More</p></div></div>
        <footer class="story-links sources"><a class="src src-primary" data-kind="paper" href="https://arxiv.org/abs/0000.00001"><span class="src-k">Paper</span> arxiv.org</a><button type="button" class="src-more" data-src-more>+1</button></footer>
      </article>
    </section>
    <div class="filter-empty" hidden><p>Empty</p><button type="button" class="filter-clear">Clear</button></div>
    <div class="end"></div>
    <footer class="colophon">Colophon</footer>
    <nav class="issue-nav bottom"><a href="/prev">Prev</a></nav>
  </main>
</div>
<nav class="dock">
  <button type="button" class="dock-toc" data-sheet-open><span>Contents</span></button>
  <div class="dock-nav" data-dock-nav></div>
  <button type="button" class="dock-top" data-scroll-top hidden>Top</button>
</nav>
<button type="button" class="to-top" data-to-top aria-hidden="true" inert>Top</button>
<div class="scrim" data-sheet-scrim hidden></div>
<div class="sr-only" id="liveAnnounce"></div>
</body></html>`;

const INDEX_FIXTURE_HTML = `<!DOCTYPE html>
<html lang="zh-CN">
<body>
<a class="skip-link" href="#main">Skip</a>
<div class="mast-sentinel"></div>
<header class="mast">
  <div class="mast-row">
    <span class="np"><a class="np-link" href="/"><span class="ai">AI</span> Digest</a></span>
    <span class="mast-chip">19 issues</span>
    <div class="mast-tools">
      <button type="button" class="ctrl-btn search-btn" data-filter-toggle></button>
      <div class="lang-seg"><a class="ctrl-btn lang-btn" data-lang-link href="/en/">EN</a></div>
      <button type="button" class="ctrl-btn theme-btn" data-theme-toggle></button>
      <button type="button" class="ctrl-btn lock-btn" data-lock></button>
    </div>
  </div>
</header>
<div class="search-scrim" data-search-scrim hidden></div>
<div class="filter-bar filter-drawer search-panel" id="digestFilterBar" hidden aria-hidden="true" inert>
  <div class="filter-inner">
    <div class="filter-input-wrap">
      <input type="search" class="filter-input">
      <span class="filter-count">19</span>
      <button type="button" class="filter-clear" hidden>Clear</button>
      <button type="button" class="filter-close">Close</button>
    </div>
    <div class="filter-tags">
      <button type="button" class="filter-tag" data-tag="llm">LLM<span class="tag-cnt num">5</span></button>
      <button type="button" class="filter-tag filter-more" hidden>+1 more</button>
    </div>
  </div>
</div>
<main id="main" class="paper">
  <header class="folio">
    <p class="folio-kicker">ARCHIVE</p>
    <h1>Archive</h1>
    <p class="folio-dek">Daily & Weekly</p>
  </header>
  <div class="quick-bar"><a class="quick-pill" data-kind="weekly" href="/weekly/latest"><span>Latest</span><strong>W48</strong><small>20</small></a></div>
  <div class="timeline">
    <section class="month-group" data-month="2026-11">
      <h2 class="month">2026-11</h2>
      <details class="grp" data-week="2026-W48" open>
        <summary>W48 11.23 - 11.29</summary>
        <ol class="issue-list">
          <li class="archive-item" data-kind="weekly" data-tags="llm" data-search="demo">
            <a class="ai-row" href="/weekly/2026-W48"><span class="ai-date"><time>W48</time></span><span class="ai-wd">Sun</span><span class="ai-title">Headline</span><span class="ai-cnt num">20</span></a>
          </li>
        </ol>
      </details>
    </section>
  </div>
  <div class="filter-empty" hidden><p>Empty</p><button type="button" class="filter-clear">Clear</button></div>
  <footer class="colophon">Colophon</footer>
</main>
<div class="sr-only"></div>
</body></html>`;

const UNLOCK_FIXTURE_HTML = `<!DOCTYPE html>
<html lang="zh-CN">
<body>
<main id="box">
  <p><span class="ai">AI</span> Digest</p>
  <hr>
  <p>ENCRYPTED EDITION</p>
  <h1 id="pwTitle">Enter passphrase</h1>
  <form id="pwForm">
    <input id="pw" type="password">
    <label class="rm-row"><input id="rm" type="checkbox"><span>Remember</span></label>
    <button id="go" type="submit"><span class="go-lbl">Open</span><span class="spin" hidden></span></button>
    <p class="err"></p>
  </form>
  <p class="demo-hint">demo</p>
  <p class="lang-alt"><a href="/en/">EN</a></p>
</main>
</body></html>`;

const NOT_FOUND_FIXTURE_HTML = `<!DOCTYPE html>
<html lang="zh-CN">
<body>
<main class="nf">
  <p>404</p>
  <h1>Not found<span>This page isn't in any issue</span></h1>
  <nav><a href="/latest">Latest</a></nav>
</main>
</body></html>`;

function extractCssClassSelectors(css: string): Set<string> {
  // Strip comments and string literals first
  const cleaned = css
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/"(?:\\.|[^"\\])*"/g, '""')
    .replace(/'(?:\\.|[^'\\])*'/g, "''");

  const classes = new Set<string>();
  let buf = "";

  const flushSelector = (sel: string): void => {
    const trimmed = sel.trim();
    if (!trimmed || trimmed.startsWith("@")) return;
    // Match class selectors like .foo, .story--lead (exclude decimals)
    const re = /\.([A-Za-z_-][A-Za-z0-9_-]*)/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(trimmed)) !== null) {
      classes.add(m[1]);
    }
  };

  for (let i = 0; i < cleaned.length; i++) {
    const ch = cleaned[i];
    if (ch === "{") {
      const header = buf.trim();
      buf = "";
      if (/^@keyframes\b/i.test(header)) {
        let kd = 1;
        while (i + 1 < cleaned.length && kd > 0) {
          i++;
          if (cleaned[i] === "{") kd++;
          else if (cleaned[i] === "}") kd--;
        }
      } else {
        // Both top-level and nested selectors precede '{'
        flushSelector(header);
      }
    } else if (ch === "}" || ch === ";") {
      buf = "";
    } else {
      buf += ch;
    }
  }
  return classes;
}

function extractHtmlClasses(html: string): Set<string> {
  const out = new Set<string>();
  const re = /\bclass="([^"]+)"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(html)) !== null) {
    for (const token of m[1].split(/\s+/)) {
      if (token) out.add(token);
    }
  }
  return out;
}

function isAllowlistedState(cls: string): boolean {
  return (
    cls.startsWith("is-") ||
    cls === "cur" ||
    cls === "kbd-focus" ||
    cls === "show" ||
    cls === "spin" ||
    cls === "ta-l" ||
    cls === "ta-c" ||
    cls === "ta-r"
  );
}

describe("web/dist artifacts and byte budgets", () => {
  it("contains all 11 deterministic LF-only artifacts", () => {
    for (const name of EXPECTED_DIST_FILES) {
      const p = join(DIST, name);
      expect(existsSync(p), `missing ${name}`).toBe(true);
      const text = readFileSync(p, "utf8");
      expect(text.length, `${name} should be non-empty`).toBeGreaterThan(0);
      expect(text.includes("\r"), `${name} must use LF line endings`).toBe(false);
    }
  });

  it("enforces raw CSS and combined per-page JS byte budgets", () => {
    const size = (f: string) => Buffer.byteLength(readFileSync(join(DIST, f), "utf8"), "utf8");

    expect(size("reader.css")).toBeLessThanOrEqual(22528);
    expect(size("index.css")).toBeLessThanOrEqual(10240);
    expect(size("unlock.css")).toBeLessThanOrEqual(4096);
    expect(size("404.css")).toBeLessThanOrEqual(4096);

    expect(size("boot.js") + size("reader.js") + size("lock.js")).toBeLessThanOrEqual(30720);
    expect(size("boot.js") + size("index.js") + size("lock.js")).toBeLessThanOrEqual(30720);
    expect(size("boot.js") + size("unlock.js")).toBeLessThanOrEqual(30720);
  });

  it("preserves exact worker placeholder, boot theme-color, and Node CLI shebang", () => {
    const worker = readFileSync(join(DIST, "worker.js"), "utf8");
    expect((worker.match(/__WORKER_CFG_JSON__/g) || []).length).toBe(1);

    const boot = readFileSync(join(DIST, "boot.js"), "utf8");
    expect(boot).toContain("digest-theme");
    expect(boot).toContain("digest-font");
    expect(boot).toContain("#0B0D12");
    expect(boot).toContain("#F8FAFD");
    expect(boot).toContain("color-scheme");
    expect(boot).toContain("colorScheme");

    const cli = readFileSync(join(DIST, "verify_decrypt.mjs"), "utf8");
    expect(cli.startsWith("#!/usr/bin/env node\n")).toBe(true);
  });

  it("drives CSS custom properties and JS theme/motion constants from web/ui-theme.json", async () => {
    const rawTheme = readFileSync(UI_THEME_PATH, "utf8");
    const theme = loadUiTheme();
    expect(rawTheme).toBe(`${JSON.stringify(theme, null, 2)}\n`);
    expect(theme.name).toBe("Lucid Indigo");
    expect(theme.palette.dark.bg).toBe("#0B0D12");
    expect(theme.palette.light.bg).toBe("#F8FAFD");
    expect(theme.palette.dark.accent).toBe("#6895FF");
    expect(theme.palette.light.accent).toBe("#2952CC");

    const readerCss = readFileSync(join(DIST, "reader.css"), "utf8");
    expect(readerCss.toLowerCase()).toContain("#0b0d12");
    expect(readerCss.toLowerCase()).toContain("#f8fafd");
    expect(readerCss.toLowerCase()).toContain("#6895ff");
    expect(readerCss.toLowerCase()).toContain("#2952cc");

    const readerJs = readFileSync(join(DIST, "reader.js"), "utf8");
    expect(readerJs).toContain("startViewTransition");
    expect(readerJs).toContain("::view-transition-new(root)");
    expect(readerJs).toContain("#0B0D12");
    expect(readerJs).toContain("#F8FAFD");

    const customTheme: UiThemeConfig = {
      ...theme,
      palette: {
        dark: {
          ...theme.palette.dark,
          bg: "#050816",
          accent: "#38BDF8",
          ctaBg: "#E2E8F0",
        },
        light: {
          ...theme.palette.light,
          bg: "#F0F6FF",
          accent: "#0284C7",
          ctaBg: "#082F49",
        },
      },
      motion: {
        ...theme.motion,
        ease: "cubic-bezier(0.25, 1, 0.5, 1)",
        pressMs: 155,
        fastMs: 175,
        baseMs: 215,
        mastMs: 235,
        cardMs: 265,
        drawerMs: 310,
        pageTransitionMs: 135,
        themeRevealMs: 520,
        themeRevealDelayMs: 35,
        arrivedHighlightMs: 1450,
        shortJumpMaxMs: 245,
        searchDebounceMs: 145,
        copyDoneHoldMs: 1950,
      },
    };

    const customRoot = renderThemeRootCss(customTheme);
    expect(customRoot).toContain("--bg:light-dark(#f0f6ff,#050816)");
    expect(customRoot).toContain("--cta-bg:light-dark(#082f49,#e2e8f0)");
    expect(customRoot).toContain("--ease:cubic-bezier(.25,1,.5,1)");
    expect(customRoot).toContain("--dur-drawer:.31s");

    const customCss = await buildCss({ theme: customTheme, write: false });
    const customJs = await buildJs({ theme: customTheme, write: false });

    for (const bundleName of ["reader.css", "index.css", "unlock.css", "404.css"]) {
      const code = customCss[bundleName]?.code.toLowerCase() ?? "";
      expect(code).toContain("#050816");
      expect(code).toContain("#f0f6ff");
      expect(code).toContain("#38bdf8");
      expect(code).toContain("#0284c7");
      expect(code).toContain("cubic-bezier(.25,1,.5,1)");
      expect(code).toContain("135ms");
    }
    expect(customCss["reader.css"]?.code).toContain("215ms");
    expect(customCss["reader.css"]?.code).toContain(".31s");
    expect(customCss["reader.css"]?.code).toContain("1.45s");
    expect(customCss["unlock.css"]?.code.toLowerCase()).toContain("#e2e8f0");
    expect(customCss["unlock.css"]?.code.toLowerCase()).toContain("#082f49");

    expect(customJs.outputs["boot.js"]).toContain("#050816");
    expect(customJs.outputs["boot.js"]).toContain("#F0F6FF");
    expect(customJs.outputs["reader.js"]).toContain("#050816");
    expect(customJs.outputs["reader.js"]).toContain("#F0F6FF");
    expect(customJs.outputs["reader.js"]).toContain("520");
    expect(customJs.outputs["reader.js"]).toContain("delay:35");
    expect(customJs.outputs["reader.js"]).toContain("1450");
    expect(customJs.outputs["reader.js"]).toContain("245");
    expect(customJs.outputs["reader.js"]).toContain("145");
    expect(customJs.outputs["reader.js"]).toContain("1950");
  });

  it("removes hyphens:auto everywhere and enforces border-radius:0 on partial-border rules", () => {
    for (const cssFile of ["reader.css", "index.css", "unlock.css", "404.css"]) {
      const css = readFileSync(join(DIST, cssFile), "utf8");
      expect(css).not.toMatch(/hyphens\s*:\s*auto/i);
    }
    const readerCss = readFileSync(join(DIST, "reader.css"), "utf8");
    expect(readerCss).toMatch(/border-radius:0/);
    expect(readerCss).toContain(".story");
    expect(readerCss).toContain(".story--lead");
    expect(readerCss).toContain(".sec-head");
    expect(readerCss).toContain(".look,.detail,.extra");
  });

  it("includes table alignment classes, keeps source links nowrap, and scopes reduced-motion TOC opacity to mobile", () => {
    const readerCss = readFileSync(join(DIST, "reader.css"), "utf8");

    // Table alignment utility classes replace inline style="text-align:..."
    expect(readerCss).toContain(".ta-l{text-align:left}");
    expect(readerCss).toContain(".ta-c{text-align:center}");
    expect(readerCss).toContain(".ta-r{text-align:right}");

    // Source links must never wrap char-by-char
    expect(readerCss).toMatch(/\.story-links a[^}]*white-space:nowrap/);
    expect(readerCss).toMatch(/\.story-links\{[^}]*overflow-wrap:normal/);
    expect(readerCss).not.toMatch(/\.story-links a\{[^}]*overflow-wrap:anywhere/);

    // Bottom issue nav keeps 2-column row even below 640px
    expect(readerCss).toMatch(/\.issue-nav\.bottom\{[^}]*grid:auto\/1fr 1fr/);

    // Top-level prefers-reduced-motion MUST NOT set opacity:0 on desktop .toc-side
    const topReducedRe =
      /@media\(prefers-reduced-motion:reduce\)\{([^{}]*(?:\{[^{}]*\}[^{}]*)*)\}/g;
    const mobileIdx = readerCss.indexOf("@media(max-width:1023px)");
    expect(mobileIdx).toBeGreaterThan(-1);
    const beforeMobile = readerCss.slice(0, mobileIdx);
    for (const m of beforeMobile.matchAll(topReducedRe)) {
      expect(m[0]).not.toContain(".toc-side{opacity:0");
    }
    // Mobile block still contains reduced-motion fade for bottom sheet
    const mobileSlice = readerCss.slice(mobileIdx, readerCss.indexOf("@media(min-width:1024px)"));
    expect(mobileSlice).toContain("@media(prefers-reduced-motion:reduce)");
    expect(mobileSlice).toContain(".toc-side{opacity:0");

    // Balanced wide editorial shell aligned with TopBar (348 + 44 + 920 + 48 = 1360px), flush TOC rail & calm stream
    expect(readerCss).toContain("min(1360px,90vw)");
    expect(readerCss).toContain("clamp(304px,23vw,348px) minmax(0,920px)");
    expect(readerCss).not.toContain("text-wrap:balance");
    expect(readerCss).toContain(".folio-sections{display:none}");
    expect(readerCss).toContain("scrollbar-color:var(");
    expect(readerCss).not.toContain("grid-column:3/6");

    // Smooth CSS Grid accordions, centered elevated search dialog, sticky TOC top, zero raw CSS nesting & zero transition:all
    expect(readerCss).toContain("@view-transition{navigation:auto}");
    expect(readerCss).toContain("@view-transition{navigation:none}");
    expect(readerCss).toContain("::view-transition-group(root){animation:none}");
    expect(readerCss).toContain("view-transition-name:mast");
    expect(readerCss).toContain(".ts-panel{display:grid;grid:0fr/1fr");
    expect(readerCss).toContain(".search-scrim{position:fixed");
    expect(readerCss).toContain(".toc-top{position:sticky;top:0");
    expect(readerCss).toContain(".sr-only{position:fixed");
    expect(readerCss).toContain("::-webkit-search-cancel-button");
    expect(readerCss).toContain(".is-arrived{");
    expect(readerCss).toContain(".to-top{position:fixed");
    expect(readerCss).not.toMatch(/&[.:[\s]/);
    expect(readerCss).not.toMatch(/transition\s*:\s*all\b/i);

    const indexCss = readFileSync(join(DIST, "index.css"), "utf8");
    expect(indexCss).toContain("min(1360px,90vw)");
    expect(indexCss).not.toMatch(/&[.:[\s]/);
    expect(indexCss).not.toMatch(/transition\s*:\s*all\b/i);
  });
});

describe("zero orphan CSS class selectors across all 4 page types", () => {
  const cases = [
    { css: "reader.css", html: READER_FIXTURE_HTML, js: "reader.js" },
    { css: "index.css", html: INDEX_FIXTURE_HTML, js: "index.js" },
    { css: "unlock.css", html: UNLOCK_FIXTURE_HTML, js: "unlock.js" },
    { css: "404.css", html: NOT_FOUND_FIXTURE_HTML, js: null },
  ] as const;

  for (const c of cases) {
    it(`${c.css} has zero orphan class selectors`, () => {
      const cssText = readFileSync(join(DIST, c.css), "utf8");
      const jsText = c.js ? readFileSync(join(DIST, c.js), "utf8") : "";
      const htmlClasses = extractHtmlClasses(c.html);
      const cssClasses = extractCssClassSelectors(cssText);

      const orphans: string[] = [];
      for (const cls of cssClasses) {
        if (htmlClasses.has(cls)) continue;
        if (isAllowlistedState(cls)) continue;
        if (jsText.includes(cls)) continue;
        orphans.push(cls);
      }
      expect(orphans, `orphan CSS classes in ${c.css}`).toEqual([]);
    });
  }
});
