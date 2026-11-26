import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

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
<header class="mast">
  <div class="mast-row">
    <h1 class="np"><a class="np-link" href="/" data-brand-home><span class="ai">AI</span> Digest</a></h1>
    <span class="edition" data-kind="daily">Daily</span>
    <span class="mast-sep" aria-hidden="true"></span>
    <nav class="issue-nav top"><a href="/prev" rel="prev"><span data-short-date>11.25</span></a></nav>
    <div class="mast-tools">
      <button type="button" class="ctrl-btn search-btn" data-filter-toggle aria-expanded="false" aria-controls="digestFilterBar"><span>Search</span><kbd>/</kbd></button>
      <div class="lang-seg"><span aria-current="page">ZH</span><a class="ctrl-btn lang-btn" data-lang-link href="/en/">EN</a></div>
      <button type="button" class="ctrl-btn theme-btn" data-theme-toggle></button>
      <button type="button" class="ctrl-btn lock-btn" data-lock></button>
    </div>
  </div>
  <div class="filter-bar filter-drawer" id="digestFilterBar" hidden>
    <div class="filter-row">
      <input type="search" class="filter-input">
      <span class="filter-count">1</span>
      <button type="button" class="filter-clear" hidden>Clear</button>
      <button type="button" class="filter-close">Close</button>
    </div>
    <div class="filter-tags">
      <button type="button" class="filter-tag" data-tag="llm">LLM <span class="num">1</span></button>
      <button type="button" class="filter-tag filter-more" hidden>+1 more</button>
    </div>
  </div>
</header>
<div class="read-prog"><div class="read-prog-fill"></div></div>
<div class="reader-shell">
  <nav class="toc-side" id="toc-sheet">
    <div class="toc-handle"></div>
    <div class="toc-head"><span>Contents</span><button type="button" class="toc-expand-all">All</button><button type="button" class="toc-close">X</button></div>
    <div class="toc-front"><a href="#front-must">Must</a></div>
    <div class="ts-group" data-sec="sec-1">
      <button type="button" class="ts-sec" aria-expanded="true" aria-controls="ts-list-sec-1">
        <span class="ts-no num">01</span><span class="ts-title">Models</span><span class="ts-cnt num" data-total="1">1</span>
      </button>
      <ol class="ts-items" id="ts-list-sec-1"><li><a href="#item-1">Title<span class="dot-hot"></span></a></li></ol>
    </div>
  </nav>
  <main id="main" class="prose-col">
    <header class="folio">
      <p class="folio-kicker">DAILY</p>
      <h1>Issue Title</h1>
      <p class="folio-dek">Summary</p>
      <span class="mast-meta"><span>2026-11-26</span><span>10 items</span><span class="motto">Motto</span></span>
      <nav class="folio-sections"><a href="#sec-1">01 Models</a></nav>
    </header>
    <section class="front front-must" id="front-must">
      <h2>Must Read</h2>
      <ol class="must-list"><li><span class="must-n num">01</span><div class="must-body"><a class="must-title" href="#item-1">Lead</a><p class="must-dek">Dek</p></div></li></ol>
    </section>
    <section class="front front-lead" id="front-themes">
      <div class="themes"><div class="theme-item"><h4>Theme</h4><p>Text <a href="#item-1">#1</a></p></div></div>
    </section>
    <section class="sec" id="sec-1">
      <div class="sec-head"><span class="sec-no num">01</span><h2>Models</h2><p class="sec-note">Note</p></div>
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
        <div class="look"><span class="look-k">Why</span><div class="prose"><p><span class="math math-wrap"><span class="frac">1/2</span><span class="sqrt">x</span></span> <code class="math-raw">$raw$</code></p></div></div>
        <div class="detail"><span class="detail-k">Details</span><div class="prose"><p>Detail</p></div></div>
        <div class="extra"><span class="extra-k">More</span><div class="prose"><p>More</p></div></div>
        <div class="story-links"><a href="https://example.com">example.com</a></div>
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
<div class="scrim" data-sheet-scrim hidden></div>
<div class="sr-only" id="liveAnnounce"></div>
</body></html>`;

const INDEX_FIXTURE_HTML = `<!DOCTYPE html>
<html lang="zh-CN">
<body>
<a class="skip-link" href="#main">Skip</a>
<header class="mast">
  <div class="mast-row">
    <h1 class="np"><a class="np-link" href="/"><span class="ai">AI</span> Digest</a></h1>
    <span class="mast-chip">19 issues</span>
    <div class="mast-tools">
      <button type="button" class="ctrl-btn search-btn" data-filter-toggle></button>
      <div class="lang-seg"><a class="ctrl-btn lang-btn" data-lang-link href="/en/">EN</a></div>
      <button type="button" class="ctrl-btn theme-btn" data-theme-toggle></button>
      <button type="button" class="ctrl-btn lock-btn" data-lock></button>
    </div>
  </div>
  <div class="filter-bar filter-drawer" id="digestFilterBar" hidden>
    <div class="filter-row">
      <input type="search" class="filter-input">
      <span class="filter-count">19</span>
      <button type="button" class="filter-clear" hidden>Clear</button>
      <button type="button" class="filter-close">Close</button>
    </div>
    <div class="filter-tags">
      <button type="button" class="filter-tag" data-tag="llm">LLM <span class="num">5</span></button>
      <button type="button" class="filter-tag filter-more" hidden>+1 more</button>
    </div>
  </div>
</header>
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

    expect(size("reader.css")).toBeLessThanOrEqual(15500);
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
    expect(boot).toContain("#141311");
    expect(boot).toContain("#faf8f4");

    const cli = readFileSync(join(DIST, "verify_decrypt.mjs"), "utf8");
    expect(cli.startsWith("#!/usr/bin/env node\n")).toBe(true);
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
