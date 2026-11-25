#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Convert an AI digest markdown file into one self-contained HTML page.

Usage:
    python3 render_html.py input.md output.html [--lang en|zh-CN] [--md-en input.en.md]

The UI language comes from --lang, else $DIGEST_LANG, else "en" (see i18n.py).
When --md-en is given, both versions are bundled into one bilingual HTML page.
"""

from __future__ import annotations

import html
import re
import sys
from dataclasses import dataclass, field
from datetime import date
from pathlib import Path

import i18n
from version import __version__

try:
    import markdown as mdlib

    _MD = mdlib.Markdown(extensions=["nl2br", "sane_lists"])
    _MD.preprocessors.deregister("html_block")
    _MD.inlinePatterns.deregister("html")
    _HAS_MD = True
except Exception:  # pragma: no cover - fallback when the library is absent
    _MD = None
    _HAS_MD = False

# Third-party notice: the CSS design tokens below (colour/radius/easing custom
# properties and the font stacks) are adapted from the AIHOT project
# (https://github.com/KKKKhazix/AIHOT, apps/web/app/app.css), MIT License,
# Copyright (c) 2026 数字生命卡兹克. Full license text: licenses/AIHOT-MIT.txt
# (see THIRD_PARTY_NOTICES.md). The AIHOT name and logo are not used.
CSS = r"""
/* Design tokens adapted from AIHOT (MIT, (c) 2026 数字生命卡兹克); see THIRD_PARTY_NOTICES.md */
:root {
  --font-sans: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI",
    "PingFang SC", "Hiragino Sans GB", "HarmonyOS Sans SC", "Microsoft YaHei",
    "Noto Sans CJK SC", "Noto Sans CJK TC", "Noto Sans", sans-serif;
  --font-mono: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Monaco, Consolas,
    "Liberation Mono", monospace;
  --radius-mark: 5px;
  --radius-control: 8px;
  --radius-card: 12px;
  --radius-panel: 14px;
  --paper-max: 1600px;
  --gutter: max(16px, env(safe-area-inset-left));
  --ease: cubic-bezier(0.25, 1, 0.5, 1);
  --ease-out: cubic-bezier(0.22, 1, 0.36, 1);
  --spring: cubic-bezier(0.34, 1.56, 0.64, 1);
  --spring-snappy: cubic-bezier(0.22, 1.35, 0.36, 1);
  --mast-h: 58px;
  --bg: #faf8f4;
  --bg-sunk: #f2eee7;
  --bg-muted: #e8e2d8;
  --well: #f3efe8;
  --surface: #ffffff;
  --ink: #1f1c18;
  --ink-2: #363029;
  --ink-3: #635a4f;
  --ink-4: #7b7164;
  --line: #e4ddd2;
  --line-strong: #d4cabd;
  --line-soft: #ede7dd;
  --accent: #b85432;
  --accent-ink: #9c4223;
  --accent-soft: rgba(184, 84, 50, 0.10);
  --accent-softer: rgba(184, 84, 50, 0.055);
  --accent-contrast: #ffffff;
  --hot: #c23b2b;
  --hot-soft: rgba(194, 59, 43, 0.10);
  --amber: #b8812d;
  --amber-ink: #84591a;
  --amber-soft: rgba(184, 129, 45, 0.12);
  --ok: #2e7856;
  --ok-soft: rgba(46, 120, 86, 0.10);
  --ok-ink: #256447;
  --prose-ink: #2b2620;
  --code-bg: #f4f0e9;
  --selection: rgba(184, 84, 50, 0.18);
  --shadow-soft: 0 4px 14px rgba(31, 24, 16, 0.06);
  color-scheme: light;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) { color-scheme: dark; }
}
:root[data-theme="dark"],
:root:not([data-theme="light"]):is(.dark-scheme) {
  color-scheme: dark;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #141311;
    --bg-sunk: #1c1a17;
    --bg-muted: #26231f;
    --well: #1b1916;
    --surface: #1f1d19;
    --ink: #efeae2;
    --ink-2: #d8d1c7;
    --ink-3: #a89f92;
    --ink-4: #877f73;
    --line: rgba(239, 234, 226, 0.10);
    --line-strong: rgba(239, 234, 226, 0.17);
    --line-soft: rgba(239, 234, 226, 0.06);
    --accent: #df7d56;
    --accent-ink: #ec9672;
    --accent-soft: rgba(223, 125, 86, 0.14);
    --accent-softer: rgba(223, 125, 86, 0.07);
    --accent-contrast: #161310;
    --hot: #e3725b;
    --hot-soft: rgba(227, 114, 91, 0.14);
    --amber: #deb668;
    --amber-ink: #e5c37e;
    --amber-soft: rgba(222, 182, 104, 0.14);
    --ok: #6cc49d;
    --ok-soft: rgba(108, 196, 157, 0.14);
    --ok-ink: #88d3b2;
    --prose-ink: #ded8ce;
    --code-bg: #181613;
    --selection: rgba(223, 125, 86, 0.26);
    --shadow-soft: 0 6px 20px rgba(0, 0, 0, 0.42);
  }
}
:root[data-theme="dark"] {
  --bg: #141311;
  --bg-sunk: #1c1a17;
  --bg-muted: #26231f;
  --well: #1b1916;
  --surface: #1f1d19;
  --ink: #efeae2;
  --ink-2: #d8d1c7;
  --ink-3: #a89f92;
  --ink-4: #877f73;
  --line: rgba(239, 234, 226, 0.10);
  --line-strong: rgba(239, 234, 226, 0.17);
  --line-soft: rgba(239, 234, 226, 0.06);
  --accent: #df7d56;
  --accent-ink: #ec9672;
  --accent-soft: rgba(223, 125, 86, 0.14);
  --accent-softer: rgba(223, 125, 86, 0.07);
  --accent-contrast: #161310;
  --hot: #e3725b;
  --hot-soft: rgba(227, 114, 91, 0.14);
  --amber: #deb668;
  --amber-ink: #e5c37e;
  --amber-soft: rgba(222, 182, 104, 0.14);
  --ok: #6cc49d;
  --ok-soft: rgba(108, 196, 157, 0.14);
  --ok-ink: #88d3b2;
  --prose-ink: #ded8ce;
  --code-bg: #181613;
  --selection: rgba(223, 125, 86, 0.26);
  --shadow-soft: 0 6px 20px rgba(0, 0, 0, 0.42);
}
* { box-sizing: border-box; }
html {
  scroll-behavior: smooth;
  -webkit-text-size-adjust: 100%;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  text-autospace: normal;
}
body {
  margin: 0;
  background: color-mix(in srgb, var(--bg) 60%, var(--surface));
  color: var(--ink);
  font-family: var(--font-sans);
  font-size: 14px;
  line-height: 1.5;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) body { background: var(--bg); }
}
:root[data-theme="dark"] body { background: var(--bg); }
::selection { background: var(--selection); }
:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
  border-radius: var(--radius-mark);
}
a { color: inherit; text-decoration: none; }
button { cursor: pointer; font-family: inherit; }
.num { font-variant-numeric: tabular-nums; }
.skip {
  position: absolute;
  left: -999px;
  top: 0;
  background: var(--surface);
  color: var(--ink);
  padding: 8px 12px;
  z-index: 20;
}
.skip:focus { left: 16px; top: 12px; }

.paper {
  width: min(var(--paper-max), calc(100% - 2 * max(var(--gutter), 5vw)));
  margin: 0 auto;
  padding: 20px 0 64px;
}
@media (min-width: 700px) {
  .paper { padding: 36px 0 80px; }
}

/* Masthead */
.mast-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  font-size: 12px;
  color: var(--ink-4);
}
.mast-top .motto {
  display: none;
  letter-spacing: 0.3em;
}
@media (min-width: 640px) {
  .mast-top .motto { display: inline; }
}
.mast-tools {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
}

.mast-row {
  display: flex;
  align-items: stretch;
  justify-content: space-between;
  gap: 20px;
  padding: 24px 0;
}
@media (min-width: 880px) {
  .mast-row { gap: 40px; padding: 32px 0; }
}
.nameplate { min-width: 0; display: flex; flex-direction: column; justify-content: center; }
.nameplate h1 {
  margin: 0;
  font-size: clamp(36px, 7vw, 72px);
  font-weight: 900;
  line-height: 0.95;
  letter-spacing: -0.04em;
  color: var(--ink);
}
.nameplate .ai { color: var(--accent); }
.imprint {
  margin: 12px 0 0;
  font-size: 11.5px;
  letter-spacing: 0.36em;
  color: var(--ink-4);
}
@media (min-width: 880px) {
  .imprint { margin-top: 16px; font-size: 12.5px; }
}

/* imprint */
.mark {
  display: flex;
  flex-shrink: 0;
  align-items: stretch;
  background: var(--well);
  box-shadow: inset 0 0 0 1px var(--line-soft);
  border-radius: var(--radius-panel);
  overflow: hidden;
}
.mark-date {
  width: 112px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 12px 8px;
}
@media (min-width: 880px) {
  .mark-date { width: 150px; padding: 16px 8px; }
}
.mark-kicker {
  font-size: 11px;
  letter-spacing: 0.2em;
  color: var(--ink-4);
}
.mark-figure {
  margin-top: 8px;
  font-size: 44px;
  font-weight: 900;
  line-height: 0.95;
  letter-spacing: -0.04em;
  color: var(--ink);
  white-space: nowrap;
}
@media (min-width: 880px) {
  .mark-figure { font-size: 64px; }
}
.mark-top { margin-top: 8px; font-size: 11.5px; color: var(--ink-2); }
.mark-bottom { font-size: 11.5px; color: var(--ink-4); }


/* Kickers */
.kicker {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.3em;
  color: var(--accent);
}
.kicker .bar {
  display: inline-block;
  width: 24px;
  height: 2px;
  border-radius: 999px;
  background: var(--accent);
}

/* Front page */
.front {
  display: grid;
}
@media (min-width: 880px) {
  .front {
    grid-template-columns: minmax(0, 1fr) 300px;
  }
}
@media (min-width: 1040px) {
  .front { grid-template-columns: minmax(0, 1fr) 340px; }
}
.front-lead {
  min-width: 0;
  padding: 28px 0;
}
@media (min-width: 880px) {
  .front-lead {
    padding: 40px 40px 40px 0;
    border-right: 1px solid var(--line);
  }
}
.front-lead .themes { margin-top: 16px; }
.theme-item { padding: 14px 0; border-bottom: 1px solid var(--line); }
.theme-item:last-child { border-bottom: 0; }
.theme-item .body {
  font-size: 16.5px;
  line-height: 1.9;
  color: var(--ink-2);
}
@media (min-width: 880px) {
  .theme-item .body { font-size: 17.5px; }
}
@media (min-width: 560px) {
  .theme-item .body { text-align: left; }
}
.theme-item p { margin: 0; }
.theme-item p + p { margin-top: 0.7em; }
.front-aside {
  min-width: 0;
  padding: 28px 0 0;
  border-top: 1px solid var(--line);
}
@media (min-width: 880px) {
  .front-aside {
    padding: 40px 0 40px 32px;
    border-top: 0;
  }
}
.must-list, .toc-sections, .toc-items-list { list-style: none; margin: 0; padding: 0; }
.must-list { margin-top: 8px; }
.must-list li a,
.must-list li .must-row {
  display: flex;
  gap: 14px;
  padding: 16px 0;
  border-bottom: 1px solid var(--line);
}
.must-n {
  width: 24px;
  flex-shrink: 0;
  font-size: 26px;
  font-weight: 900;
  line-height: 0.95;
  letter-spacing: -0.03em;
  color: var(--accent);
}
.must-title {
  display: block;
  font-size: 15px;
  font-weight: 700;
  line-height: 1.55;
  color: var(--ink);
}
.must-dek {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  margin-top: 6px;
  font-size: 13px;
  line-height: 1.65;
  color: var(--ink-3);
}
.must-list a:hover .must-title { color: var(--accent); }

.toc-block { margin-top: 32px; }
.toc-block:first-child { margin-top: 0; }
.toc-sections { margin-top: 12px; }
.toc-sections a {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 6px 0;
  font-size: 13.5px;
}
.toc-sections .n {
  width: 28px;
  flex-shrink: 0;
  font-size: 14px;
  font-weight: 700;
  color: var(--ink);
}
.toc-sections .lab {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--ink-2);
}
.toc-sections a:hover .lab { color: var(--accent); }
.toc-sections .cnt { flex-shrink: 0; font-size: 12px; color: var(--ink-4); }

.toc-group { margin-top: 18px; }
.toc-group-label {
  display: block;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.12em;
  color: var(--ink-4);
  margin-bottom: 4px;
}
.toc-items-list a {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 5px 0;
  font-size: 13.5px;
  color: var(--ink-2);
}
.toc-items-list a:hover { color: var(--accent); }
.toc-items-list .n {
  width: 1.6em;
  flex-shrink: 0;
  font-weight: 700;
  color: var(--ink);
  font-size: 13px;
}
.toc-note, .toc-empty { margin: 10px 0 0; font-size: 12px; color: var(--ink-4); }

/* Daily-only: full-width contents under the masthead */
.toc-wide { padding: 28px 0 8px; }
@media (min-width: 760px) {
  .toc-wide .toc-cols {
    columns: 2;
    column-gap: 48px;
    column-rule: 1px solid var(--line);
  }
  .toc-wide .toc-group { break-inside: avoid; }
}

/* Section pages */
.sec {
  scroll-margin-top: 24px;
  padding-top: 48px;
}
@media (min-width: 880px) {
  .sec { padding-top: 64px; }
}
.sec-head {
  display: flex;
  align-items: baseline;
  gap: 12px;
  border-bottom: 1px solid var(--line-strong);
  padding-bottom: 12px;
}
@media (min-width: 880px) {
  .sec-head { gap: 16px; }
}
.sec-no {
  font-size: 26px;
  font-weight: 900;
  line-height: 1;
  letter-spacing: -0.03em;
  color: var(--accent);
}
@media (min-width: 880px) {
  .sec-no { font-size: 30px; }
}
.sec-head h2 {
  margin: 0;
  min-width: 0;
  font-size: 24px;
  font-weight: 900;
  line-height: 1.25;
  letter-spacing: -0.02em;
  color: var(--ink);
}
@media (min-width: 880px) {
  .sec-head h2 { font-size: 28px; }
}
.subhead {
  margin: 28px 0 0;
  padding: 16px 0 8px;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.3em;
  color: var(--accent);
  display: flex;
  align-items: center;
  gap: 10px;
}
.subhead .bar {
  width: 24px;
  height: 2px;
  border-radius: 999px;
  background: var(--accent);
}
.sec-note {
  margin: 20px 0 0;
  padding: 16px 0;
  color: var(--ink-3);
  font-size: 15px;
  border-bottom: 1px solid var(--line);
}

/* Stories — newspaper entries, not boxed cards */
.story {
  display: flex;
  flex-direction: column;
  min-width: 0;
  padding: 24px 0;
  border-bottom: 1px solid var(--line);
  scroll-margin-top: 24px;
  overflow-wrap: anywhere;
}
.story:target h3 { color: var(--accent); }
.story-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--ink-3);
}
.story-meta .source { color: var(--ink-3); min-width: 0; }
.story-meta .when {
  margin-left: auto;
  flex-shrink: 0;
  color: var(--ink-4);
}
.badge {
  display: inline-flex;
  height: 18px;
  align-items: center;
  padding: 0 8px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 500;
  line-height: 1;
  background: var(--bg-sunk);
  color: var(--ink-3);
  border: 1px solid var(--line-soft);
}
.badge-accent { background: var(--accent-soft); color: var(--accent); border-color: transparent; }
.badge-hot { background: var(--hot-soft); color: var(--hot); border-color: transparent; }
.badge-amber { background: var(--amber-soft); color: var(--amber-ink); border-color: transparent; }
.story h3 {
  margin: 12px 0 0;
  font-size: 19px;
  font-weight: 700;
  line-height: 1.5;
  letter-spacing: -0.01em;
  color: var(--ink);
  text-wrap: pretty;
}
@media (min-width: 880px) {
  .story h3 { font-size: 20px; }
}
.story-no {
  margin-right: 0.3em;
  font-weight: 700;
  color: var(--ink-4);
  font-variant-numeric: tabular-nums;
}
.dek {
  margin: 8px 0 0;
  font-size: 15px;
  line-height: 1.85;
  color: var(--ink-2);
}
@media (min-width: 560px) {
  .dek { text-align: left; }
}
.dek p, .look p, .detail p, .extra p { margin: 0; }
.dek p + p, .look p + p, .detail p + p, .extra p + p { margin-top: 0.7em; }
.look {
  margin: 10px 0 0;
  font-size: 15px;
  line-height: 1.85;
  color: var(--ink-2);
}
@media (min-width: 560px) {
  .look { text-align: left; }
}
.look-k, .detail-k, .extra-k {
  margin-right: 8px;
  font-weight: 600;
  color: var(--accent);
}
.detail-k, .extra-k {
  display: block;
  margin: 0 0 6px;
  color: var(--ink-4);
  font-size: 12px;
  letter-spacing: 0.16em;
  font-weight: 600;
}
.detail, .extra {
  margin: 14px 0 0;
  font-size: 15px;
  line-height: 1.85;
  color: var(--prose-ink);
}
@media (min-width: 560px) {
  .detail { text-align: left; }
}
.detail strong, .dek strong, .look strong { color: var(--ink); font-weight: 650; }
.detail a, .dek a, .look a, .extra a, .theme-item a {
  color: var(--accent);
  text-decoration: underline;
  text-decoration-thickness: 1px;
  text-underline-offset: 0.18em;
}
.detail a:hover, .dek a:hover { text-decoration-color: var(--accent-ink); }
code {
  font-family: var(--font-mono);
  font-size: 0.86em;
  background: var(--code-bg);
  padding: 0.05em 0.35em;
  border-radius: 4px;
}

.story-links {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 16px;
  margin-top: auto;
  padding-top: 12px;
}
.story-links a {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  min-height: 28px;
  font-size: 12.5px;
  font-weight: 500;
  color: var(--ink-3);
  transition: color 120ms;
}
.story-links a::after {
  content: "↗";
  font-size: 11px;
  margin-left: 1px;
}
.story-links a:hover { color: var(--accent); }

.colophon {
  padding: 40px 0 0;
  text-align: center;
}
.colophon .end {
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0.6em;
  color: var(--ink-4);
}
.colophon p {
  margin: 12px 0 0;
  font-size: 12px;
  color: var(--ink-4);
}

.to-top {
  position: fixed;
  right: max(16px, env(safe-area-inset-right));
  bottom: max(16px, env(safe-area-inset-bottom));
  z-index: 30;
  width: 38px;
  height: 38px;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: color-mix(in srgb, var(--surface) 90%, transparent);
  -webkit-backdrop-filter: blur(10px);
  backdrop-filter: blur(10px);
  color: var(--ink-3);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 15px;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.45), var(--shadow-soft);
  opacity: 0;
  transform: translateY(12px) scale(0.86);
  pointer-events: none;
  transition: transform 280ms var(--spring), opacity 180ms ease, color 140ms ease, border-color 140ms ease;
}
.to-top.show {
  opacity: 1;
  transform: translateY(0) scale(1);
  pointer-events: auto;
}
.to-top:hover {
  color: var(--accent);
  border-color: var(--accent);
  transform: translateY(-2px) scale(1.04);
}
.to-top:active { transform: translateY(0) scale(0.92); transition-duration: 90ms; }

@media (max-width: 639px) {
  .mark-figure { font-size: 40px; }
  .nameplate h1 { font-size: 40px; }
  .front-lead { padding-top: 24px; }
}

.edition-index {
  padding: 20px 0 4px;
  border-bottom: 1px solid var(--line);
}
.edition-index .toc-block { margin-top: 0; }
@media (min-width: 640px) {
  .edition-index .toc-sections {
    display: flex;
    flex-wrap: wrap;
    gap: 0 36px;
  }
  .edition-index .toc-sections a { padding: 10px 0; }
}
@media print {
  body { background: #fff; }
  header.mast { position: static; backdrop-filter: none; transform: none !important; }
  .to-top, .skip, .theme-switch, .lang-switch, .ctrl-btn, .read-prog, .copy-link { display: none !important; }
  .story { break-inside: avoid; }
  a { color: inherit; }
}
@media (prefers-reduced-motion: reduce) {
  html { scroll-behavior: auto; }
  *, *::before, *::after { transition: none !important; animation: none !important; }
}

/* Weekly front page, stacked bands (2026-11-16) */
.front.front-stack { display: block; }
.front-stack .front-must { padding: 28px 0 8px; border-bottom: 1px solid var(--line); }
.front-stack .front-must .must-list li { break-inside: avoid; }
@media (min-width: 640px) {
  .front-stack .front-must .must-list { columns: 2; column-gap: 36px; column-rule: 1px solid var(--line); }
}
@media (min-width: 1040px) {
  .front-stack .front-must { padding-top: 36px; }
  .front-stack .front-must .must-list { columns: 3; column-gap: 40px; }
}
.front-stack .front-lead { padding: 28px 0 12px; border-right: 0; }
.front-stack .front-lead .themes { orphans: 2; widows: 2; }
@media (min-width: 1040px) {
  .front-stack .front-lead { padding: 32px 0 16px; }
  .front-stack .front-lead .themes { columns: 2; column-gap: 48px; column-rule: 1px solid var(--line); }
  .front-stack .theme-item:first-child { padding-top: 0; }
}
/* Compact masthead: full-bleed topbar sits above .paper */
.paper { padding-top: 16px; }
@media (min-width: 700px) { .paper { padding-top: 22px; } }
.mast-bar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 9px 0 8px;
  min-width: 0;
}
.mast-bar .np {
  margin: 0;
  font-size: 22px;
  font-weight: 900;
  line-height: 1;
  letter-spacing: -0.03em;
  color: var(--ink);
  white-space: nowrap;
}
@media (min-width: 880px) { .mast-bar .np { font-size: 28px; } }
.mast-bar .np .ai { color: var(--accent); }
.mast-chip {
  display: inline-flex;
  align-items: baseline;
  gap: 5px;
  flex-shrink: 0;
  padding: 3px 9px;
  background: var(--well);
  box-shadow: inset 0 0 0 1px var(--line-soft);
  border-radius: 999px;
  white-space: nowrap;
}
.mast-chip .mark-kicker { font-size: 10.5px; letter-spacing: 0.15em; color: var(--ink-4); }
.mast-chip .mark-figure {
  margin: 0;
  font-size: 15px;
  font-weight: 900;
  line-height: 1;
  letter-spacing: -0.02em;
  color: var(--ink);
}
.mast-meta {
  display: none;
  min-width: 0;
  gap: 14px;
  font-size: 12px;
  color: var(--ink-4);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.mast-meta .motto { display: none; letter-spacing: 0.3em; }
@media (min-width: 640px) { .mast-meta { display: inline-flex; } }
@media (min-width: 1000px) { .mast-meta .motto { display: inline; } }
.mast-bar .mast-tools { display: inline-flex; align-items: center; flex-shrink: 0; margin-left: auto; font-size: 12px; color: var(--ink-4); gap: 8px; }
@media (max-width: 639px) {
  .mast-bar { gap: 8px; }
  .mast-bar .edition { display: none; }
}
/* Sidebar contents (2026-11-17): replaces the full-width top contents */
.layout { display: block; }
.toc-side-toggle, .toc-side-backdrop { display: none; }
.toc-side {
  position: fixed;
  z-index: 40;
  top: 0; bottom: 0; left: 0;
  width: min(320px, 86vw);
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 18px 18px 28px;
  background: var(--surface);
  border-right: 1px solid var(--line-strong);
  box-shadow: var(--shadow-soft);
  transform: translateX(-102%);
  visibility: hidden;
  transition: transform .22s var(--ease), visibility 0s linear .22s;
}
body.toc-open .toc-side { transform: none; visibility: visible; transition: transform .22s var(--ease); }
.toc-side-backdrop {
  position: fixed; inset: 0; z-index: 35;
  background: rgba(0, 0, 0, 0.32);
  opacity: 0; pointer-events: none;
  transition: opacity .2s;
}
body.toc-open .toc-side-backdrop { display: block; opacity: 1; pointer-events: auto; }
.toc-side-toggle {
  position: fixed; z-index: 30;
  left: 16px; bottom: 16px;
  height: 36px; padding: 0 14px;
  align-items: center; gap: 6px;
  border: 1px solid var(--line-strong);
  border-radius: 999px;
  background: var(--surface);
  color: var(--ink-2);
  font-size: 13px; font-weight: 600; letter-spacing: 0.08em;
  box-shadow: var(--shadow-soft);
  display: inline-flex;
}
.toc-side-toggle:hover { color: var(--accent); }
.toc-side-close {
  display: inline-flex; float: right;
  width: 28px; height: 28px; margin: -4px -6px 0 0;
  align-items: center; justify-content: center;
  border: 0; background: none; color: var(--ink-3); font-size: 18px;
}
.toc-side-head { margin-bottom: 10px; }
.ts-sec { margin-top: 14px; }
.ts-sec:first-of-type { margin-top: 4px; }
.ts-sec-link {
  display: flex; align-items: baseline; gap: 8px;
  padding: 5px 0; font-size: 13.5px; font-weight: 700; color: var(--ink);
}
.ts-sec-link .n { width: 22px; flex-shrink: 0; font-size: 12px; color: var(--ink-4); font-weight: 600; }
.ts-sec-link .lab { flex: 1; min-width: 0; }
.ts-sec-link .cnt { font-size: 11.5px; font-weight: 400; color: var(--ink-4); }
.ts-sublabel { display: block; margin: 8px 0 2px 30px; font-size: 11.5px; letter-spacing: 0.1em; color: var(--ink-4); }
.ts-items { list-style: none; margin: 0 0 0 0; padding: 0; }
.ts-items a {
  display: flex; align-items: baseline; gap: 6px;
  margin-left: 8px;
  padding: 4px 0 4px 8px;
  border-left: 2px solid transparent;
  font-size: 12.5px; line-height: 1.45; color: var(--ink-3);
}
.ts-items .n { flex-shrink: 0; min-width: 1.7em; font-size: 11.5px; font-weight: 700; color: var(--ink-4); }
.ts-items .t { min-width: 0; overflow-wrap: break-word; }
.ts-items .badge { flex-shrink: 0; align-self: center; height: 16px; padding: 0 6px; font-size: 10.5px; } /* the "new" badge stays on the title's line (review 2026-11-18) */
.ts-items a:hover, .ts-sec-link:hover .lab { color: var(--accent); }
.toc-side a.cur { color: var(--accent); }
.ts-items a.cur { border-left-color: var(--accent); background: var(--accent-softer); }
.ts-items a.cur .n { color: var(--accent); }
.toc-side-note { margin: 12px 0 0; font-size: 11.5px; color: var(--ink-4); }
.sec, .story, [id^="item-"], [id^="en-item-"] { scroll-margin-top: 16px; }
@media (min-width: 1024px) {
  .layout {
    display: grid;
    grid-template-columns: 248px minmax(0, 1fr);
    column-gap: 48px;
    align-items: start;
  }
  .toc-side {
    position: sticky; z-index: auto;
    top: 12px; bottom: auto; left: auto;
    width: auto;
    max-height: calc(100vh - 24px);
    padding: 8px 12px 16px 0;
    background: none; border-right: 0; box-shadow: none;
    transform: none; visibility: visible; transition: none;
    scrollbar-width: thin;
  }
  .toc-side-close, .toc-side-toggle, .toc-side-backdrop { display: none !important; }
  .layout-body { min-width: 0; }
  .layout-body .front-stack .front-must .must-list { columns: 2; }
}
@media (min-width: 1480px) {
  .layout-body .front-stack .front-must .must-list { columns: 3; }
}
/* layout2: wide-screen reading measure.
   Page/container width unchanged (90%, max 1600px). Body text 16px from 1024px,
   17px from 1600px; the article column is capped at 1100px and the sidebar+article
   pair is centered inside the paper, so the page still feels full. */
@media (min-width: 1024px) {
  .layout { grid-template-columns: 248px minmax(0, 1100px); justify-content: center; }
  .dek, .look, .detail, .extra { font-size: 16px; }
}
@media (min-width: 1600px) {
  .dek, .look, .detail, .extra { font-size: 17px; line-height: 1.8; }
}
@media print { .toc-side, .toc-side-toggle, .toc-side-backdrop, .filter-bar, .filter-empty { display: none !important; } .layout { display: block; } .sec[hidden], .story[hidden], .front[hidden] { display: block !important; } }

/* Full-bleed sticky auto-hide TopBar (Zhihu-style) + physics-spring buttons + reading perf */
.mast {
  position: sticky;
  top: 0;
  z-index: 30;
  width: 100%;
  background: color-mix(in srgb, var(--surface) 88%, transparent);
  -webkit-backdrop-filter: blur(16px) saturate(160%);
  backdrop-filter: blur(16px) saturate(160%);
  border-bottom: 1px solid var(--line-soft);
  transition: transform 280ms var(--ease-out), background-color 180ms ease,
              box-shadow 200ms ease, border-color 200ms ease;
  will-change: transform;
}
.mast > .mast-bar,
.mast > .issue-nav.top {
  width: min(var(--paper-max), calc(100% - 2 * max(var(--gutter), 5vw)));
  margin-inline: auto;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) .mast {
    background: color-mix(in srgb, var(--bg-sunk) 88%, transparent);
  }
}
:root[data-theme="dark"] .mast {
  background: color-mix(in srgb, var(--bg-sunk) 88%, transparent);
}
.mast.is-scrolled {
  background: color-mix(in srgb, var(--surface) 92%, transparent);
  border-bottom-color: var(--line);
  box-shadow: 0 2px 14px rgba(31, 24, 16, 0.08);
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) .mast.is-scrolled {
    background: color-mix(in srgb, var(--bg-sunk) 92%, transparent);
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.42);
  }
}
:root[data-theme="dark"] .mast.is-scrolled {
  background: color-mix(in srgb, var(--bg-sunk) 92%, transparent);
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.42);
}
.mast.is-hidden {
  transform: translateY(calc(-100% - 2px));
  box-shadow: none !important;
  pointer-events: none;
}
.read-prog {
  position: absolute;
  left: 0;
  right: 0;
  bottom: -1px;
  height: 2px;
  overflow: hidden;
  pointer-events: none;
  opacity: 0;
  transition: opacity 180ms ease;
}
.mast.is-scrolled .read-prog { opacity: 1; }
.read-prog-fill {
  display: block;
  width: 100%;
  height: 100%;
  background: linear-gradient(90deg, var(--accent), color-mix(in srgb, var(--accent) 55%, var(--amber)));
  transform-origin: 0 50%;
  transform: scaleX(0);
  will-change: transform;
}

/* Physics-spring animated control buttons (merged Lang & Theme toggles) */
@keyframes ctrlPop {
  0% { transform: scale(0.92); }
  55% { transform: scale(1.045); }
  100% { transform: scale(1); }
}
.ctrl-btn {
  position: relative;
  overflow: hidden;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  height: 30px;
  padding: 0 11px;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: color-mix(in srgb, var(--surface) 88%, var(--well));
  color: var(--ink-2);
  font-size: 12px;
  font-weight: 560;
  letter-spacing: 0.01em;
  white-space: nowrap;
  text-decoration: none;
  user-select: none;
  -webkit-tap-highlight-color: transparent;
  --btn-glint: rgba(255, 255, 255, 0.55);
  box-shadow: inset 0 1px 0 var(--btn-glint), 0 1px 2px rgba(31, 24, 16, 0.05);
  transition: transform 280ms var(--spring), border-color 160ms ease,
              background-color 160ms ease, color 160ms ease, box-shadow 200ms ease;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) .ctrl-btn {
    --btn-glint: rgba(255, 255, 255, 0.07);
    box-shadow: inset 0 1px 0 var(--btn-glint), 0 1px 3px rgba(0, 0, 0, 0.32);
  }
}
:root[data-theme="dark"] .ctrl-btn {
  --btn-glint: rgba(255, 255, 255, 0.07);
  box-shadow: inset 0 1px 0 var(--btn-glint), 0 1px 3px rgba(0, 0, 0, 0.32);
}
.ctrl-btn::after {
  content: "";
  position: absolute;
  inset: 0;
  background: linear-gradient(115deg, transparent 22%, rgba(255, 255, 255, 0.28) 50%, transparent 78%);
  transform: translateX(-125%);
  transition: transform 520ms var(--ease-out);
  pointer-events: none;
}
.ctrl-btn:hover {
  color: var(--accent-ink);
  border-color: color-mix(in srgb, var(--accent) 48%, var(--line));
  background: color-mix(in srgb, var(--accent) 8%, var(--surface));
  transform: translateY(-1px);
  box-shadow: inset 0 1px 0 var(--btn-glint), 0 4px 12px color-mix(in srgb, var(--accent) 18%, transparent);
}
.ctrl-btn:hover::after { transform: translateX(125%); }
.ctrl-btn:active {
  transform: translateY(0) scale(0.93);
  transition-duration: 90ms;
}
.ctrl-btn.is-pop { animation: ctrlPop 340ms var(--spring); }
.ctrl-ic {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  color: var(--accent);
  transition: transform 380ms var(--spring);
}
.lang-btn:hover .ctrl-ic { transform: rotate(28deg); }
.lang-btn[data-active-lang="en"] .ctrl-ic { transform: rotate(180deg); }
.lang-btn[data-active-lang="en"]:hover .ctrl-ic { transform: rotate(208deg); }
.slot-roll {
  display: inline-block;
  height: 16px;
  line-height: 16px;
  overflow: hidden;
  text-align: center;
}
.slot-track {
  display: flex;
  flex-direction: column;
  transition: transform 360ms var(--spring);
}
.slot-v {
  height: 16px;
  line-height: 16px;
  display: block;
}
.lang-btn[data-active-lang="en"] .slot-track { transform: translateY(-50%); }

.theme-ics {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  color: var(--accent);
}
.theme-ics svg {
  position: absolute;
  inset: 0;
  opacity: 0;
  transform: scale(0.4) rotate(-65deg);
  transition: transform 360ms var(--spring), opacity 180ms ease;
}
.theme-btn[data-mode="system"] .ic-sys,
.theme-btn[data-mode="light"] .ic-sun,
.theme-btn[data-mode="dark"] .ic-moon {
  opacity: 1;
  transform: scale(1) rotate(0deg);
}
.ctrl-lbl { line-height: 1; }
@media (max-width: 479px) {
  .ctrl-btn { height: 28px; padding: 0 9px; font-size: 11.5px; }
  .theme-btn .ctrl-lbl { display: none; }
  .theme-btn { width: 30px; padding: 0; }
}

/* Story card layout containment (no content-visibility size-skipping that breaks fast-scroll TOC) */
article.story {
  contain: layout style;
  border-radius: var(--radius-control);
  transition: background-color 180ms ease;
}
article.story.kbd-focus {
  outline: 2px solid color-mix(in srgb, var(--accent) 55%, transparent);
  outline-offset: 6px;
}
.story-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
}
.story-head h3 { flex: 1 1 auto; min-width: 0; }
.copy-link {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 26px;
  padding: 0 7px;
  margin-top: 2px;
  border: 1px solid transparent;
  border-radius: 999px;
  background: transparent;
  color: var(--ink-4);
  font-size: 11.5px;
  opacity: 0.48;
  transition: opacity 150ms ease, color 150ms ease, border-color 150ms ease,
              background-color 150ms ease, transform 260ms var(--spring);
}
article.story:hover .copy-link,
.copy-link:focus-visible,
.copy-link.is-copied { opacity: 1; }
.copy-link:hover {
  color: var(--accent-ink);
  border-color: var(--line);
  background: var(--well);
  transform: translateY(-1px);
}
.copy-link:active { transform: scale(0.92); transition-duration: 80ms; }
.copy-link .copy-tip { display: none; font-weight: 560; }
.copy-link .ic-chk { display: none; }
.copy-link.is-copied {
  color: var(--ok-ink);
  border-color: color-mix(in srgb, var(--ok) 40%, var(--line));
  background: var(--ok-soft);
  animation: ctrlPop 320ms var(--spring);
}
.copy-link.is-copied .ic-lnk { display: none; }
.copy-link.is-copied .ic-chk,
.copy-link.is-copied .copy-tip { display: inline; }
@media (hover: none) { .copy-link { opacity: 0.72; } }

/* Scroll offsets & calm desktop TOC sidebar (never squeezed by TopBar) */
.sec, .story, [id^="item-"], [id^="en-item-"] { scroll-margin-top: calc(var(--mast-h, 76px) + 16px); }
@media (min-width: 1024px) {
  .toc-side {
    top: 18px;
    max-height: calc(100vh - 36px);
  }
}
.toc-side-toggle {
  bottom: max(16px, env(safe-area-inset-bottom));
  left: max(16px, env(safe-area-inset-left));
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
  background: color-mix(in srgb, var(--surface) 90%, transparent);
  transition: transform 280ms var(--spring), border-color 150ms ease, color 150ms ease;
}
.toc-side-toggle:hover { transform: translateY(-1px); border-color: var(--accent); color: var(--accent-ink); }
.toc-side-toggle:active { transform: scale(0.94); transition-duration: 80ms; }

/* Client-side keyword + tag filter bar */
.filter-bar {
  margin: 14px 0 18px;
  padding: 10px 12px;
  background: var(--well);
  box-shadow: inset 0 0 0 1px var(--line-soft);
  border-radius: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.filter-row { display: flex; align-items: center; gap: 8px; position: relative; }
.filter-input {
  flex: 1 1 auto;
  min-width: 0;
  height: 34px;
  padding: 0 34px 0 11px;
  border: 1px solid var(--line);
  border-radius: 9px;
  background: var(--surface);
  color: var(--ink);
  font-family: inherit;
  font-size: 13.5px;
  outline: none;
  transition: border-color 150ms ease, box-shadow 180ms ease;
}
.filter-input::placeholder { color: var(--ink-4); }
.filter-input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 16%, transparent);
}
.filter-kbd {
  position: absolute;
  right: 9px;
  top: 50%;
  transform: translateY(-50%);
  padding: 1px 6px;
  border: 1px solid var(--line);
  border-radius: 5px;
  background: var(--bg-sunk);
  color: var(--ink-4);
  font-family: var(--font-mono);
  font-size: 11px;
  line-height: 1.4;
  pointer-events: none;
  transition: opacity 140ms ease;
}
.filter-row:focus-within .filter-kbd,
.filter-bar.is-active .filter-kbd { opacity: 0; }
@media (hover: none), (max-width: 639px) { .filter-kbd { display: none; } }
.filter-count { flex: 0 0 auto; font-size: 12px; color: var(--ink-4); padding: 0 4px; }
.filter-clear {
  flex: 0 0 auto;
  height: 26px;
  padding: 0 10px;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--surface);
  color: var(--ink-3);
  font-size: 12px;
  transition: transform 240ms var(--spring), color 140ms ease, border-color 140ms ease;
}
.filter-clear:hover { color: var(--ink); border-color: var(--accent); transform: translateY(-1px); }
.filter-clear:active { transform: scale(0.93); }
.filter-tags { display: flex; flex-wrap: wrap; gap: 5px; }
@media (max-width: 639px) {
  .filter-tags {
    flex-wrap: nowrap;
    overflow-x: auto;
    -webkit-overflow-scrolling: touch;
    scrollbar-width: none;
    padding-bottom: 2px;
    mask-image: linear-gradient(90deg, #000 92%, transparent);
    -webkit-mask-image: linear-gradient(90deg, #000 92%, transparent);
  }
  .filter-tags::-webkit-scrollbar { display: none; }
}
.filter-tag {
  flex: 0 0 auto;
  height: 25px;
  padding: 0 10px;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--surface);
  color: var(--ink-3);
  font-size: 11.5px;
  white-space: nowrap;
  transition: transform 240ms var(--spring), color 140ms ease, border-color 140ms ease, background-color 140ms ease;
}
.filter-tag:hover { color: var(--accent-ink); border-color: var(--accent); transform: translateY(-1px); }
.filter-tag:active { transform: scale(0.93); }
.filter-tag[aria-pressed="true"] {
  background: var(--accent-softer);
  border-color: var(--accent);
  color: var(--accent);
  font-weight: 600;
}
.story-meta .badge[data-tag] {
  cursor: pointer;
  transition: transform 220ms var(--spring), border-color 140ms ease, color 140ms ease;
}
.story-meta .badge[data-tag]:hover { border-color: var(--accent); color: var(--accent); transform: translateY(-1px); }
.filter-empty { margin: 24px 0; color: var(--ink-4); font-size: 14px; text-align: center; }

/* English (Latin) typography: the wide letter-spacing above is tuned for CJK kickers. */
html[lang="en"] .mast-top .motto, html[lang="en"] .imprint, html[lang="en"] .mark-kicker,
html[lang="en"] .kicker, html[lang="en"] .toc-group-label, html[lang="en"] .subhead,
html[lang="en"] .look-k, html[lang="en"] .detail-k, html[lang="en"] .extra-k, html[lang="en"] .colophon .end,
html[lang="en"] .mast-meta .motto, html[lang="en"] .ts-sublabel, html[lang="en"] .ts-sec-link,
.lang-en .mast-top .motto, .lang-en .imprint, .lang-en .mark-kicker,
.lang-en .kicker, .lang-en .toc-group-label, .lang-en .subhead,
.lang-en .look-k, .lang-en .detail-k, .lang-en .extra-k, .lang-en .colophon .end,
.lang-en .mast-meta .motto, .lang-en .ts-sublabel, .lang-en .ts-sec-link { letter-spacing: 0.06em; }
html[lang="en"] .kicker, html[lang="en"] .mark-kicker, html[lang="en"] .look-k, html[lang="en"] .detail-k,
html[lang="en"] .extra-k, html[lang="en"] .toc-group-label,
.lang-en .kicker, .lang-en .mark-kicker, .lang-en .look-k, .lang-en .detail-k,
.lang-en .extra-k, .lang-en .toc-group-label { text-transform: uppercase; font-size: 11px; }
html[lang="en"] .nameplate h1, .lang-en .nameplate h1 { letter-spacing: -0.03em; }
html[lang="en"] .story h3, .lang-en .story h3 { text-wrap: balance; }
html[lang="en"] .dek, html[lang="en"] .look, html[lang="en"] .detail, html[lang="en"] .extra,
.lang-en .dek, .lang-en .look, .lang-en .detail, .lang-en .extra { hyphens: auto; line-height: 1.65; overflow-wrap: break-word; }

/* Single-page bilingual panes */
html[data-lang="en"] .lang-zh { display: none !important; }
html:not([data-lang="en"]) .lang-en { display: none !important; }
.lang-switch { margin-right: 2px; }
@media print {
  .mast { position: static; margin: 0; padding: 0; background: none; box-shadow: none; border: 0; }
  .read-prog, .ctrl-btn, .copy-link, .filter-kbd { display: none !important; }
}

"""

ITEM_RE = re.compile(
    r"^###\s+(?:🆕\s+)?(\d+(?:[–-]\d+)?)\.\s+(.*)$"
)
FIELD_RE = re.compile(
    r"^\*\*(" + "|".join(re.escape(x) for x in sorted(i18n.FIELD_ALIASES, key=len, reverse=True))
    + r")\s*[：:]\*\*\s*(.*)$"
)
TOC_ITEM_RE = re.compile(
    r"(🆕\s*)?(\d+(?:[–-]\d+)?)\.\s*(.+?)(?=(?:\s+🆕\s*|\s+)\d+(?:[–-]\d+)?\.|$)"
)
RELATED_RE = re.compile(
    r"((?:" + "|".join(i18n.RELATED_WORDS) + r")\s*[：:])((?:\s*[0-9]+(?:\s*[–—-]\s*[0-9]+)?\s*[、,，]?)+)"
)
NUM_RANGE_RE = re.compile(r"(\d+)(?:\s*[–—-]\s*(\d+))?")
LINK_RE = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")
CODE_RE = re.compile(r"`([^`]+)`")
BOLD_RE = re.compile(r"\*\*(.+?)\*\*")
FRONT_LABELS = i18n.FRONT_ALIASES


@dataclass
class Item:
    number: str
    title: str
    is_new: bool = False
    meta: str = ""
    fields: dict[str, str] = field(default_factory=dict)
    extra: str = ""
    links: list[tuple[str, str]] = field(default_factory=list)


@dataclass
class TocEntry:
    number: str
    title: str
    is_new: bool = False


@dataclass
class TocGroup:
    label: str
    entries: list[TocEntry] = field(default_factory=list)
    note: str = ""


@dataclass
class Section:
    title: str
    note: str = ""
    blocks: list = field(default_factory=list)  # Item | ("sub", title) | ("note", text)


@dataclass
class Digest:
    title: str
    period: str = ""
    must: str = ""
    themes: str = ""
    toc_groups: list[TocGroup] = field(default_factory=list)
    toc_note: str = ""
    sections: list[Section] = field(default_factory=list)
    items: list[Item] = field(default_factory=list)


def _load_css() -> str:
    # Prefer the constant; if a sibling file exists during development, use it
    # only when the constant is still a placeholder.
    text = CSS.strip()
    if len(text) < 40:
        bundled = Path(__file__).resolve().parent / "_digest_styles.css"
        for candidate in (bundled, Path("/tmp/digest-build/digest.css")):
            if candidate.exists():
                return candidate.read_text()
    return text if text else "body{font-family:sans-serif;max-width:820px;margin:auto;padding:24px;}"


def md_to_html(text: str, inline: bool = False) -> str:
    text = (text or "").strip()
    if not text:
        return ""
    if _HAS_MD:
        html_out = _MD.reset().convert(text)
    else:
        html_out = _fallback_markdown(text)
    html_out = _decorate_links(html_out)
    if inline:
        html_out = _unwrap_single_p(html_out)
    return html_out


_CTRL_WS_RE = re.compile(r"[\x00-\x20\x7f]+")
_URL_SCHEME_RE = re.compile(r"^([A-Za-z][A-Za-z0-9+.-]*):")
_ANCHOR_RE = re.compile(r'<a\s[^>]*href="([^"]*)"[^>]*>(.*?)</a>', re.S | re.I)


def _is_safe_url(url: str) -> bool:
    cleaned = _CTRL_WS_RE.sub("", html.unescape(url or "")).strip()
    if not cleaned or cleaned.startswith("//"):
        return False
    if cleaned.startswith("#"):
        return True
    m = _URL_SCHEME_RE.match(cleaned)
    if not m:
        return True
    return m.group(1).lower() in ("http", "https")


def _fallback_markdown(text: str) -> str:
    placeholders: list[str] = []

    def stash(chunk: str) -> str:
        placeholders.append(chunk)
        return f"\x00PH{len(placeholders) - 1}\x00"

    def codes(m: re.Match) -> str:
        return stash(f"<code>{html.escape(m.group(1))}</code>")

    def links(m: re.Match) -> str:
        label, url = m.group(1), m.group(2)
        if not _is_safe_url(url):
            return stash(html.escape(label))
        return stash(
            f'<a href="{html.escape(url, quote=True)}" target="_blank" '
            f'rel="noopener noreferrer">{html.escape(label)}</a>'
        )

    work = CODE_RE.sub(codes, text)
    work = LINK_RE.sub(links, work)
    work = html.escape(work)
    work = BOLD_RE.sub(r"<strong>\1</strong>", work)
    for i, chunk in enumerate(placeholders):
        work = work.replace(f"\x00PH{i}\x00", chunk)
    parts = re.split(r"\n\s*\n", work.strip())
    out = []
    for part in parts:
        out.append("<p>" + part.replace("\n", "<br>\n") + "</p>")
    return "".join(out)


def _decorate_links(src: str) -> str:
    def repl(m: re.Match) -> str:
        full, href, inner = m.group(0), m.group(1), m.group(2)
        if not _is_safe_url(href):
            return inner
        open_tag = full[: full.index(">") + 1]
        if "target=" in open_tag:
            return full
        return open_tag[:-1] + ' target="_blank" rel="noopener noreferrer">' + inner + "</a>"

    return _ANCHOR_RE.sub(repl, src)


def _unwrap_single_p(src: str) -> str:
    m = re.fullmatch(r"<p>(.*)</p>", src.strip(), re.S)
    return m.group(1) if m else src


def item_anchor(number: str) -> str:
    return "item-" + re.sub(r"[^0-9A-Za-z]+", "-", number).strip("-")


def parse_digest(text: str, lang: str | None = None) -> Digest:
    lang = i18n.resolve_lang(lang)
    lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    # Drop a trailing "dropped this week" section if a source file still has one.
    for i, line in enumerate(lines):
        if line.strip() in i18n.DROPPED_MARKERS:
            lines = lines[:i]
            break

    idx = 0
    while idx < len(lines) and not lines[idx].strip():
        idx += 1
    if idx >= len(lines):
        raise ValueError("empty markdown")

    raw_title = lines[idx].strip()
    raw_title = re.sub(r"^#+\s*", "", raw_title)
    raw_title = raw_title.strip("*").strip()
    title, period = raw_title, ""
    if "·" in raw_title:
        left, right = raw_title.split("·", 1)
        title, period = left.strip(), right.strip()
    idx += 1

    digest = Digest(title=title, period=period)
    front: dict[str, list[str]] = {"must": [], "themes": [], "toc": []}
    current_front: str | None = None

    def is_body_start(s: str) -> bool:
        return s.startswith("## ") or s.startswith("### ") or s.startswith("#### ")

    while idx < len(lines):
        raw = lines[idx]
        s = raw.strip()
        if is_body_start(s):
            break
        if s in {"---", ""}:
            idx += 1
            continue
        key = s.strip("*").strip()
        if key in FRONT_LABELS:
            current_front = FRONT_LABELS[key]
            idx += 1
            continue
        if current_front:
            front[current_front].append(raw)
        idx += 1

    digest.must = "\n".join(front["must"]).strip()
    digest.themes = "\n".join(front["themes"]).strip()
    toc_text = "\n".join(front["toc"]).strip()
    digest.toc_groups, digest.toc_note = _parse_toc(toc_text)

    sections: list[Section] = []
    current: Section | None = None
    item: Item | None = None
    body: list[str] = []

    def flush_item() -> None:
        nonlocal item, body
        if item is None:
            return
        _fill_item(item, body)
        if current is not None:
            current.blocks.append(item)
        digest.items.append(item)
        item, body = None, []

    def ensure_section(title: str) -> Section:
        nonlocal current
        if current is None or current.title != title:
            current = Section(title=title)
            sections.append(current)
        return current

    while idx < len(lines):
        s = lines[idx].strip()
        if s == "---":
            flush_item()
            idx += 1
            continue
        if s.startswith("## "):
            flush_item()
            current = Section(title=s[3:].strip())
            sections.append(current)
            idx += 1
            continue
        if s.startswith("#### "):
            flush_item()
            ensure_section(i18n.t(lang, "sec_default_papers") if current is None else current.title)
            current.blocks.append(("sub", s[5:].strip()))
            idx += 1
            continue
        m = ITEM_RE.match(s)
        if m:
            flush_item()
            ensure_section(i18n.t(lang, "sec_default_items") if current is None else current.title)
            title_text = m.group(2).strip()
            is_new = "🆕" in s.split(".", 1)[0] or title_text.startswith("🆕")
            title_text = re.sub(r"^🆕\s*", "", title_text)
            item = Item(number=m.group(1), title=title_text, is_new=is_new)
            body = []
            idx += 1
            continue
        if item is not None:
            body.append(lines[idx])
        elif current is not None and s:
            current.note = (current.note + "\n" + s).strip() if current.note else s
        idx += 1

    flush_item()
    digest.sections = sections
    if not digest.sections or not digest.items:
        raise ValueError(
            f"malformed digest markdown: parsed {len(digest.sections)} sections and {len(digest.items)} items "
            "(expected '## Section' headings and numbered '### 1. Title' items)"
        )
    if not digest.toc_groups and digest.items:
        digest.toc_groups = [
            TocGroup(
                label="",
                entries=[
                    TocEntry(it.number, it.title, it.is_new) for it in digest.items
                ],
            )
        ]
    return digest


def _parse_toc(text: str) -> tuple[list[TocGroup], str]:
    if not text:
        return [], ""
    groups: list[TocGroup] = []
    notes: list[str] = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            continue
        if line.startswith("（") or line.startswith("("):
            notes.append(line)
            continue
        label = ""
        rest = line
        if line.startswith("**") and "**" in line[2:]:
            end = line.index("**", 2)
            label = line[2:end].strip()
            rest = line[end + 2 :].strip()
        entries: list[TocEntry] = []
        for part in re.split(r"[\u3000]+|\s{2,}|\n", rest):
            m = re.match(r"^(🆕\s*)?(\d+(?:[–-]\d+)?)\.\s*(.+)$", part.strip())
            if not m:
                continue
            entries.append(
                TocEntry(
                    number=m.group(2),
                    title=m.group(3).strip(),
                    is_new=bool(m.group(1)) or "🆕" in m.group(3),
                )
            )
        title_clean = [e.title.replace("🆕", "").strip() for e in entries]
        for e, t in zip(entries, title_clean):
            e.title = t
        if entries:
            groups.append(TocGroup(label=label, entries=entries))
        elif rest:
            groups.append(TocGroup(label=label or "", entries=[], note=rest))
        elif label:
            groups.append(TocGroup(label=label, entries=[], note=""))
    return groups, "\n".join(notes)


def _fill_item(item: Item, body: list[str]) -> None:
    pending_field: str | None = None
    buckets: dict[str, list[str]] = {}
    extra: list[str] = []

    def add(field: str | None, line: str) -> None:
        if field:
            buckets.setdefault(field, []).append(line)
        else:
            extra.append(line)

    i = 0
    # Optional meta line in backticks, possibly after a blank line.
    while i < len(body) and not body[i].strip():
        i += 1
    if i < len(body):
        s = body[i].strip()
        if s.startswith("`") and s.endswith("`") and s.count("`") == 2:
            item.meta = s[1:-1].strip()
            i += 1

    while i < len(body):
        line = body[i]
        s = line.strip()
        if s.startswith("🔗"):
            item.links.extend(_parse_links(s))
            i += 1
            continue
        fm = FIELD_RE.match(s)
        if fm:
            pending_field = i18n.FIELD_ALIASES[fm.group(1)]
            add(pending_field, fm.group(2))
            i += 1
            continue
        if not s:
            add(pending_field, "")
            i += 1
            continue
        add(pending_field, line)
        i += 1

    for key, parts in buckets.items():
        item.fields[key] = "\n".join(parts).strip()
    item.extra = "\n".join(extra).strip()
    # If 🔗 was not used, collect leftover markdown links into item.links later if needed.


def _parse_links(line: str) -> list[tuple[str, str]]:
    return [(m.group(1).strip(), m.group(2).strip()) for m in LINK_RE.finditer(line)]


def _link_related(src: str, id_prefix: str = "") -> str:
    def repl(m: re.Match) -> str:
        label, body = m.group(1), m.group(2)
        bits: list[str] = []
        cursor = 0
        for n in NUM_RANGE_RE.finditer(body):
            bits.append(html.escape(body[cursor : n.start()]))
            a, b = n.group(1), n.group(2)
            if b:
                # Ranges in 主线 usually refer to inclusive item numbers, not a combined card.
                # Link the ends individually if no combined item exists; keep a single span.
                bits.append(
                    f'<a href="#{html.escape(id_prefix + item_anchor(a))}">{html.escape(a)}</a>'
                    f"{html.escape(n.group(0)[len(a):-len(b)] if n.group(0).endswith(b) else '–')}"
                    f'<a href="#{html.escape(id_prefix + item_anchor(b))}">{html.escape(b)}</a>'
                )
            else:
                bits.append(f'<a href="#{html.escape(id_prefix + item_anchor(a))}">{html.escape(a)}</a>')
            cursor = n.end()
        bits.append(html.escape(body[cursor:]))
        return html.escape(label) + "".join(bits)

    return RELATED_RE.sub(repl, src)


YEAR = date.today().year
_FILENAME_YEAR_RE = re.compile(r"(?:^|[^0-9])(20\d{2})-\d{2}-\d{2}")
_SLASH_DATE_RE = re.compile(r"(?:(\d{4})[/-])?(\d{1,2})/(\d{1,2})")
_ISO_DATE_RE = re.compile(r"(\d{4})-(\d{1,2})-(\d{1,2})")


def _year_from_path(src: Path) -> int | None:
    m = _FILENAME_YEAR_RE.search(src.name)
    return int(m.group(1)) if m else None


def _split_meta(meta: str) -> tuple[str, str, list[str]]:
    parts = [p.strip() for p in re.split(r"\s*·\s*", meta or "") if p.strip()]
    source = parts[0] if parts else ""
    when = ""
    tags: list[str] = []
    for p in parts[1:]:
        if re.match(r"^\d{1,2}/\d{1,2}", p):
            when = re.sub(r"(\d{1,2})/(\d{1,2})", r"\1.\2", p)
        else:
            tags.append(p)
    return source, when, tags


def _badge_class(tag: str) -> str:
    if tag in i18n.BADGE_ACCENT_TAGS:
        return "badge badge-accent"
    return "badge"


_LEAD_EMOJI_RE = re.compile(r"^[\U0001F000-\U0001FAFF\u2190-\u21FF\u2300-\u27BF\u2B00-\u2BFF\uFE0F\u200D\s]+")


def _plain_section(title: str) -> str:
    # 去掉版面标题前的任意 emoji（🧠 📄 🛠 🏢 ⚡ 🛡 💬 📊 …）
    return _LEAD_EMOJI_RE.sub("", title).strip() or title


def _period_info(period: str, weekly: bool, lang: str, year: int | None = None) -> dict[str, str]:
    base_year = year if year is not None else date.today().year
    raw_matches = _SLASH_DATE_RE.findall(period or "") or _ISO_DATE_RE.findall(period or "")
    dates: list[date] = []
    for y_s, mo_s, day_s in raw_matches:
        y = int(y_s) if y_s else base_year
        mo, day = int(mo_s), int(day_s)
        if not y_s and dates and (mo, day) < (dates[-1].month, dates[-1].day):
            y = dates[-1].year + 1
        try:
            dates.append(date(y, mo, day))
        except ValueError:
            continue
    motto = i18n.t(lang, "motto")
    edition = i18n.t(lang, "edition_weekly" if weekly else "edition_daily")
    if weekly and len(dates) >= 2:
        a, b = dates[0], dates[-1]
        iso = a.isocalendar()
        return {
            "line": i18n.date_range(lang, a, b),
            "figure": str(iso.week),
            "kicker": i18n.t(lang, "kicker_weekly"),
            "top": i18n.year_week(lang, iso.year, iso.week),
            "bottom": f"{a.month:02d}.{a.day:02d} — {b.month:02d}.{b.day:02d}",
            "edition": edition,
            "motto": motto,
        }
    if dates:
        d = dates[-1]
        return {
            "line": (i18n.date_long(lang, d) + " · " + i18n.weekday(lang, d)
                     if i18n.normalize_lang(lang) == "zh-CN"
                     else i18n.weekday(lang, d) + ", " + i18n.date_long(lang, d)),
            "figure": str(d.day),
            "kicker": i18n.t(lang, "kicker_daily"),
            "top": i18n.year_month(lang, d),
            "bottom": i18n.weekday(lang, d),
            "edition": edition,
            "motto": motto,
        }
    return {
        "line": period or "",
        "figure": period or "",
        "kicker": i18n.t(lang, "kicker_other"),
        "top": "",
        "bottom": "",
        "edition": edition,
        "motto": motto,
    }


def _must_parts(text: str) -> tuple[str, str, str]:
    text = text.strip()
    m = re.match(r"\*\*(.+?)\*\*\s*(?:[（(]([^）)]+)[）)])?\s*[：:]\s*(.*)", text, re.S)
    if m:
        return m.group(1).strip(), (m.group(2) or "").strip(), m.group(3).strip()
    m = re.match(r"(.+?)[：:]\s*(.*)", text, re.S)
    if m:
        return re.sub(r"\*\*", "", m.group(1)).strip(), "", m.group(2).strip()
    return re.sub(r"\*\*", "", text).strip(), "", ""


def _guess_anchor(title: str, digest: Digest) -> str | None:
    key = re.sub(r"[（(][^）)]+[）)]", "", title).strip()
    key = re.split(r"[与：:]|\s+(?:and|&)\s+", key)[0].strip()
    for it in digest.items:
        if key and (key in it.title or it.title in title or it.title in key):
            return item_anchor(it.number)
    # distinctive shared token of length >= 4
    for it in digest.items:
        for token in re.findall(r"[A-Za-z][A-Za-z0-9.\-_+]{3,}|[\u4e00-\u9fff]{3,}", key):
            if token in it.title:
                return item_anchor(it.number)
    return None


def _nameplate_html(title: str) -> str:
    if title.startswith("AI"):
        return f'<span class="ai">AI</span>{html.escape(title[2:])}'
    return html.escape(title)


def _split_list_items(text: str) -> list[str]:
    items: list[str] = []
    current: list[str] = []
    for line in (text or "").splitlines():
        s = line.strip()
        if re.match(r"^(\d+[\.、)]\s+|[-*+]\s+)", s):
            if current:
                items.append("\n".join(current).strip())
            current = [re.sub(r"^(\d+[\.、)]\s+|[-*+]\s+)", "", s)]
        elif s or current:
            current.append(s)
    if current:
        items.append("\n".join(current).strip())
    return items


def render_item(item: Item, lang: str = "en", id_prefix: str = "") -> str:
    lang = i18n.normalize_lang(lang)
    anchor = id_prefix + item_anchor(item.number)
    source, when, tags = _split_meta(item.meta)
    meta_bits: list[str] = []
    if source:
        meta_bits.append(f'<span class="source">{html.escape(source)}</span>')
    for tag in tags:
        meta_bits.append(
            f'<span class="{_badge_class(tag)}" data-tag="{html.escape(tag, quote=True)}">{html.escape(tag)}</span>'
        )
    if item.is_new:
        meta_bits.append(f'<span class="badge badge-hot">{html.escape(i18n.t(lang, "new"))}</span>')
    if when:
        meta_bits.append(f'<span class="when num">{html.escape(when)}</span>')
    tags_attr = f' data-tags="{html.escape("|".join(tags), quote=True)}"' if tags else ""
    copy_lbl = html.escape(i18n.t(lang, "copy_link"), quote=True)
    copied_lbl = html.escape(i18n.t(lang, "copied"))
    copy_btn = (
        f'<button type="button" class="copy-link" data-copy-anchor="{html.escape(anchor, quote=True)}"'
        f' aria-label="{copy_lbl}" title="{copy_lbl}">'
        '<svg class="ic-lnk" viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">'
        '<path d="M6.8 9.2a3.2 3.2 0 0 0 4.5 0l1.6-1.6a3.2 3.2 0 0 0-4.5-4.5L7.5 4"/>'
        '<path d="M9.2 6.8a3.2 3.2 0 0 0-4.5 0L3.1 8.4a3.2 3.2 0 0 0 4.5 4.5l.9-.9"/></svg>'
        '<svg class="ic-chk" viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">'
        '<path d="M3.2 8.4 6.5 11.7 12.8 4.8"/></svg>'
        f'<span class="copy-tip">{copied_lbl}</span></button>'
    )
    parts = [
        f'<article class="story" id="{html.escape(anchor)}"{tags_attr}>',
        f'<div class="story-meta">{"".join(meta_bits)}</div>' if meta_bits else "",
        f'<div class="story-head"><h3><span class="story-no num">{html.escape(item.number)}.</span>{html.escape(item.title)}</h3>{copy_btn}</div>',
    ]
    if item.fields.get("dek"):
        parts.append(f'<div class="dek">{md_to_html(item.fields["dek"])}</div>')
    if item.fields.get("look"):
        parts.append(
            f'<div class="look"><span class="look-k">{html.escape(i18n.t(lang, "label_look"))}</span>'
            f'{md_to_html(item.fields["look"], inline=True)}</div>'
        )
    if item.fields.get("detail"):
        parts.append(
            f'<div class="detail"><span class="detail-k">{html.escape(i18n.t(lang, "label_detail"))}</span>'
            f"{md_to_html(item.fields['detail'])}</div>"
        )
    for key, val in item.fields.items():
        if key in {"dek", "look", "detail"} or not val:
            continue
        label = i18n.t(lang, f"label_{key}") if f"label_{key}" in i18n.STRINGS[lang] else key
        parts.append(
            f'<div class="extra"><span class="extra-k">{html.escape(label)}</span>'
            f"{md_to_html(val)}</div>"
        )
    if item.extra:
        parts.append(f'<div class="extra">{md_to_html(item.extra)}</div>')
    if item.links:
        pill_parts = []
        for label, url in item.links:
            if _is_safe_url(url):
                pill_parts.append(
                    f'<a href="{html.escape(url, quote=True)}" target="_blank" '
                    f'rel="noopener noreferrer">{html.escape(label)}</a>'
                )
            else:
                pill_parts.append(f"<span>{html.escape(label)}</span>")
        parts.append(f'<div class="story-links">{"".join(pill_parts)}</div>')
    parts.append("</article>")
    return "\n".join(p for p in parts if p)


def _section_nav(digest: Digest, lang: str = "en") -> str:
    if not digest.sections:
        return ""
    label_sections = html.escape(i18n.t(lang, "issue_sections"))
    chunks = [f'<nav class="toc-block" aria-label="{label_sections}">', f'<div class="kicker"><span class="bar"></span>{label_sections}</div>', '<ol class="toc-sections">']
    for i, sec in enumerate(digest.sections, 1):
        label = _plain_section(sec.title)
        n = sum(1 for b in sec.blocks if isinstance(b, Item))
        cnt = i18n.count_items(lang, n) if n else "—"
        chunks.append(
            f'<li><a href="#{html.escape(_section_id(sec.title))}">'
            f'<span class="n num">{i:02d}</span>'
            f'<span class="lab">{html.escape(label)}</span>'
            f'<span class="cnt num">{html.escape(cnt)}</span></a></li>'
        )
    chunks.append("</ol></nav>")
    return "\n".join(chunks)


def render_item_index(digest: Digest, lang: str = "en") -> str:
    if not digest.toc_groups and not digest.items:
        return ""
    groups = digest.toc_groups or [
        TocGroup(
            label="",
            entries=[TocEntry(it.number, it.title, it.is_new) for it in digest.items],
        )
    ]
    chunks: list[str] = []
    for group in groups:
        chunks.append('<div class="toc-group">')
        if group.label:
            chunks.append(f'<span class="toc-group-label">{html.escape(group.label)}</span>')
        if group.entries:
            chunks.append('<ol class="toc-items-list">')
            for e in group.entries:
                badge = f' <span class="badge badge-hot">{html.escape(i18n.t(lang, "new"))}</span>' if e.is_new else ""
                href = item_anchor(e.number)
                chunks.append(
                    f'<li><a href="#{html.escape(href)}"><span class="n num">'
                    f"{html.escape(e.number)}</span>"
                    f"{html.escape(e.title)}{badge}</a></li>"
                )
            chunks.append("</ol>")
        elif group.note:
            chunks.append(f'<p class="toc-empty">{html.escape(group.note)}</p>')
        chunks.append("</div>")
    if digest.toc_note:
        chunks.append(f'<p class="toc-note">{html.escape(digest.toc_note)}</p>')
    return "\n".join(chunks)


def _title_head(title: str) -> str:
    t = re.sub(r"\s+", " ", title).strip()
    m = re.search(r"[：:]", t)
    return t[: m.start()].strip() if m and m.start() >= 2 else ""


def _short_title(title: str, limit: float = 16.0, keep_rest: bool = False) -> str:
    """Sidebar summary (~16 CJK / 28 Latin columns).

    Uses the head before the first colon, unless the head is only a short
    source/org name (e.g. "Epoch", "OpenAI") or is shared by another item in
    the same issue (keep_rest); then it keeps "head: rest" and truncates.
    CJK characters count as one column, Latin characters as limit/28, so the
    same limit fits both scripts. Latin text is cut at a word boundary.
    """
    t = re.sub(r"\s+", " ", title).strip()
    latin_w = limit / 28.0

    def w(ch: str) -> float:
        return 1.0 if ord(ch) > 0x2E80 else latin_w

    head = _title_head(t)
    if head and not keep_rest and sum(w(c) for c in head) > 6.0:
        t = head

    total = 0.0
    for i, ch in enumerate(t):
        total += w(ch)
        if total > limit:
            cut = t[:i]
            if ord(ch) <= 0x2E80 and " " in cut and ord(cut[-1]) <= 0x2E80:
                head_cut = cut.rsplit(" ", 1)[0]
                if len(head_cut) >= len(cut) * 0.6:
                    cut = head_cut
            return cut.rstrip("：: ,;·-") + "…"
    return t


def render_sidebar(digest: Digest, lang: str = "en", id_prefix: str = "") -> str:
    """Table of contents as a sidebar: sections, each with its items (anchors)."""
    if not digest.sections:
        return ""
    toc_title = {}
    for g in digest.toc_groups:
        for e in g.entries:
            toc_title[e.number] = (e.title, e.is_new)
    chunks = [
        f'<button class="toc-side-toggle" id="{id_prefix}tocToggle" type="button" aria-controls="{id_prefix}tocSide" '
        f'aria-expanded="false" aria-label="{html.escape(i18n.t(lang, "toc_open"))}">{html.escape(i18n.t(lang, "toc_open_btn"))}</button>',
        f'<div class="toc-side-backdrop" id="{id_prefix}tocBackdrop"></div>',
        f'<nav class="toc-side" id="{id_prefix}tocSide" aria-label="{html.escape(i18n.t(lang, "toc"))}">',
        f'<div class="toc-side-head"><button class="toc-side-close" id="{id_prefix}tocClose" type="button" aria-label="{html.escape(i18n.t(lang, "toc_close"))}">✕</button>'
        f'<div class="kicker"><span class="bar"></span>{html.escape(i18n.t(lang, "toc"))}</div></div>',
    ]
    for i, sec in enumerate(digest.sections, 1):
        items = [b for b in sec.blocks if isinstance(b, Item)]
        label = _plain_section(sec.title)
        cnt = f"{len(items)}" if items else ""
        chunks.append('<div class="ts-sec">')
        chunks.append(
            f'<a class="ts-sec-link" href="#{html.escape(id_prefix + _section_id(sec.title))}">'
            f'<span class="n num">{i:02d}</span><span class="lab">{html.escape(label)}</span>'
            f'<span class="cnt num">{cnt}</span></a>'
        )
        if items:
            chunks.append('<ol class="ts-items">')
            for it in items:
                title, is_new = toc_title.get(it.number, (it.title, it.is_new))
                badge = f' <span class="badge badge-hot">{html.escape(i18n.t(lang, "new"))}</span>' if is_new else ""
                full = max((it.title, title), key=len)
                cls = ' class="has-new"' if is_new else ""
                chunks.append(
                    f'<li><a href="#{html.escape(id_prefix + item_anchor(it.number))}"{cls} title="{html.escape(full, quote=True)}">'
                    f'<span class="n num">{html.escape(it.number)}</span>'
                    f'<span class="t">{html.escape(title)}</span>{badge}</a></li>'
                )
            chunks.append("</ol>")
        chunks.append("</div>")
    if digest.toc_note:
        chunks.append(f'<p class="toc-side-note">{html.escape(digest.toc_note)}</p>')
    chunks.append("</nav>")
    return "\n".join(chunks)


def render_must(digest: Digest, lang: str = "en", id_prefix: str = "") -> str:
    items = _split_list_items(digest.must)
    if not items:
        return ""
    chunks = [f'<div class="kicker"><span class="bar"></span>{html.escape(i18n.t(lang, "must"))}</div>', '<ol class="must-list">']
    for i, raw in enumerate(items, 1):
        title, when, dek = _must_parts(raw)
        href = _guess_anchor(title, digest)
        dek_html = f'<span class="must-dek">{html.escape(dek)}</span>' if dek else ""
        when_bit = (f"（{html.escape(when)}）" if i18n.normalize_lang(lang) == "zh-CN" else f" ({html.escape(when)})") if when else ""
        inner = (
            f'<span class="must-n num">{i}</span>'
            f'<span class="minw"><span class="must-title">{html.escape(title)}{when_bit}</span>{dek_html}</span>'
        )
        if href:
            chunks.append(f'<li><a href="#{html.escape(id_prefix + href)}">{inner}</a></li>')
        else:
            chunks.append(f'<li><div class="must-row">{inner}</div></li>')
    chunks.append("</ol>")
    return "\n".join(chunks)


def render_themes(digest: Digest, lang: str = "en", id_prefix: str = "") -> str:
    items = _split_list_items(digest.themes)
    if not items:
        return ""
    chunks = [f'<div class="kicker"><span class="bar"></span>{html.escape(i18n.t(lang, "themes"))}</div>', '<div class="themes">']
    for raw in items:
        body = _link_related(md_to_html(raw), id_prefix=id_prefix)
        chunks.append(f'<div class="theme-item"><div class="body">{body}</div></div>')
    chunks.append("</div>")
    return "\n".join(chunks)


_FOOTER_SCRIPT = """<script>
(function(){
  const KEY='digest-theme';
  const MODES=['system','light','dark'];
  const root=document.documentElement;
  const tbtns=[...document.querySelectorAll('[data-theme-toggle]')];
  const metas=[...document.querySelectorAll('meta[name="theme-color"]')];
  const mq=window.matchMedia?matchMedia('(prefers-color-scheme: dark)'):null;
  function pop(el){
    if(!el)return;
    el.classList.remove('is-pop');
    void el.offsetWidth;
    el.classList.add('is-pop');
  }
  function paintMeta(){
    const isDark=root.getAttribute('data-theme')==='dark'||(!root.getAttribute('data-theme')&&Boolean(mq&&mq.matches));
    const col=isDark?'#141311':'#faf8f4';
    metas.forEach(m=>m.setAttribute('content',col));
  }
  let curTheme='system';
  function applyTheme(mode,animate){
    curTheme=MODES.includes(mode)?mode:'system';
    if(curTheme==='dark'||curTheme==='light')root.setAttribute('data-theme',curTheme);
    else root.removeAttribute('data-theme');
    tbtns.forEach(b=>{
      b.dataset.mode=curTheme;
      let lbls={};try{lbls=JSON.parse(b.dataset.labels||'{}');}catch(e){}
      const txt=lbls[curTheme]||curTheme;
      const sp=b.querySelector('.ctrl-lbl');
      if(sp)sp.textContent=txt;
      const base=b.dataset.ariaBase||'Appearance';
      b.setAttribute('aria-label',base+': '+txt);
      if(animate)pop(b);
    });
    paintMeta();
  }
  try{curTheme=localStorage.getItem(KEY)||'system';}catch(e){}
  applyTheme(curTheme,false);
  try{if(mq&&mq.addEventListener)mq.addEventListener('change',paintMeta);}catch(e){}
  function cycleTheme(){
    const nxt=MODES[(MODES.indexOf(curTheme)+1)%MODES.length];
    try{localStorage.setItem(KEY,nxt);}catch(e){}
    applyTheme(nxt,true);
  }
  tbtns.forEach(b=>b.addEventListener('click',cycleTheme));

  const panes=[...document.querySelectorAll('.lang-pane')];
  const scopes=panes.length?panes:[document];
  const isVisible=s=>s===document||getComputedStyle(s).display!=='none';
  const activeScope=()=>scopes.find(isVisible)||scopes[0];

  /* Zhihu-style TopBar scroll state: hide on scroll down, reveal on scroll up or 3s idle */
  let lastY=Math.max(0,scrollY),sRaf=0,stopTimer=0,accDown=0,accUp=0,barHidden=false,tocLockUntil=0;
  let setups=[];
  function setBarHidden(hide){
    const s=setups.find(x=>isVisible(x.scope));
    if(!s||!s.mast)return;
    const h=Boolean(hide&&!s.pinned()&&scrollY>64);
    barHidden=h;
    s.mast.classList.toggle('is-hidden',h);
    if(!h){clearTimeout(stopTimer);stopTimer=0;}
  }
  function armIdleReveal(){
    clearTimeout(stopTimer);
    stopTimer=setTimeout(()=>{stopTimer=0;if(barHidden)setBarHidden(false);},3000);
  }
  const unlockToc=()=>{tocLockUntil=0;};
  addEventListener('wheel',unlockToc,{passive:true});
  addEventListener('touchstart',unlockToc,{passive:true});
  addEventListener('pointerdown',e=>{if(!e.target||!e.target.closest||!e.target.closest('.toc-side'))tocLockUntil=0;},{passive:true});

  setups=scopes.map(function(scope){
    const mast=scope.querySelector('header.mast');
    const progFill=mast?mast.querySelector('.read-prog-fill'):null;
    const side=scope.querySelector('.toc-side');
    const top=scope.querySelector('.to-top');
    const links=side?[...side.querySelectorAll('a[href^="#"]')]:[];
    const targets=links.map(a=>document.getElementById(decodeURIComponent(a.getAttribute('href').slice(1))));
    let visTargets=[],targetsReady=false,mastH=76,cur=-1,close=function(){};
    let mastHover=false,mastFocus=false;

    if(mast){
      mast.addEventListener('pointerenter',e=>{if(e.pointerType==='mouse'){mastHover=true;setBarHidden(false);}});
      mast.addEventListener('pointerleave',e=>{if(e.pointerType==='mouse'){mastHover=false;}});
      mast.addEventListener('focusin',e=>{let kv=false;try{kv=Boolean(e.target&&e.target.matches&&e.target.matches(':focus-visible'));}catch(_){}if(kv){mastFocus=true;setBarHidden(false);}});
      mast.addEventListener('focusout',e=>{if(!mast.contains(e.relatedTarget))mastFocus=false;});
    }

    function syncMastH(){
      if(!mast||!isVisible(scope))return;
      const h=Math.round(mast.offsetHeight);
      if(h>0){mastH=h;root.style.setProperty('--mast-h',h+'px');}
    }

    function rebuildTargets(){
      if(!side||!isVisible(scope))return;
      visTargets=[];
      for(let i=0;i<targets.length;i++){
        const t=targets[i];
        if(!t||t.hidden||(t.closest&&t.closest('[hidden]')))continue;
        visTargets.push([i,t]);
      }
      targetsReady=true;
    }

    function setActive(hit,followSide){
      if(hit===cur)return;
      const linkEl=hit>=0?links[hit]:null;
      let nextTop=null;
      if(followSide&&linkEl&&side&&innerWidth>=1024&&side.scrollHeight>side.clientHeight){
        const r=linkEl.getBoundingClientRect(),sr=side.getBoundingClientRect();
        const pad=Math.min(56,sr.height*0.18);
        if(r.top<sr.top+pad||r.bottom>sr.bottom-pad){
          nextTop=Math.max(0,Math.min(side.scrollHeight-side.clientHeight,side.scrollTop+(r.top-sr.top)-sr.height*0.35));
        }
      }
      if(cur>=0&&links[cur])links[cur].classList.remove('cur');
      cur=hit;
      if(hit<0){if(followSide&&side)side.scrollTop=0;return;}
      if(!linkEl)return;
      linkEl.classList.add('cur');
      if(nextTop!==null)side.scrollTop=nextTop;
    }

    function reset(){
      mastHover=false;mastFocus=false;targetsReady=false;tocLockUntil=0;
      if(mast)mast.classList.remove('is-hidden');
      if(cur>=0&&links[cur])links[cur].classList.remove('cur');
      cur=-1;
    }

    function syncToc(sy,rawMax){
      if(!side||!isVisible(scope))return;
      if(Date.now()<tocLockUntil)return;
      if(!targetsReady)rebuildTargets();
      if(!visTargets.length){setActive(-1,true);return;}
      const lim=rawMax!==undefined?rawMax:(document.documentElement.scrollHeight-innerHeight);
      let hit=-1;
      if(lim>4&&sy>=lim-4){
        hit=visTargets[visTargets.length-1][0];
      }else{
        const probeY=mastH+28;
        let lo=0,hi=visTargets.length-1;
        while(lo<=hi){
          const mid=(lo+hi)>>1;
          if(visTargets[mid][1].getBoundingClientRect().top<=probeY){hit=visTargets[mid][0];lo=mid+1;}
          else hi=mid-1;
        }
      }
      setActive(hit,true);
    }

    function syncChrome(sy){
      if(!isVisible(scope))return;
      const rawMax=document.documentElement.scrollHeight-innerHeight;
      syncToc(sy,rawMax);
      const pct=rawMax>0?Math.max(0,Math.min(1,sy/rawMax)):0;
      if(progFill)progFill.style.transform='scaleX('+pct.toFixed(4)+')';
      if(mast)mast.classList.toggle('is-scrolled',sy>20);
      if(top)top.classList.toggle('show',sy>420);
    }

    if(side){
      const tg=scope.querySelector('.toc-side-toggle'),bd=scope.querySelector('.toc-side-backdrop'),cl=scope.querySelector('.toc-side-close');
      const setOpen=o=>{
        document.body.classList.toggle('toc-open',o);
        if(tg)tg.setAttribute('aria-expanded',o?'true':'false');
        if(o)setBarHidden(false);
      };
      close=()=>setOpen(false);
      if(tg)tg.addEventListener('click',()=>setOpen(true));
      if(bd)bd.addEventListener('click',()=>setOpen(false));
      if(cl)cl.addEventListener('click',()=>setOpen(false));
      addEventListener('keydown',e=>{if(e.key==='Escape')setOpen(false);});
      side.addEventListener('click',e=>{
        const a=e.target.closest('a[href^="#"]');
        if(!a)return;
        const idx=links.indexOf(a);
        if(idx>=0){tocLockUntil=Date.now()+650;setActive(idx,false);}
        setOpen(false);
      });
    }
    if(top){
      top.addEventListener('click',()=>{tocLockUntil=0;scrollTo({top:0,behavior:'smooth'});});
    }

    /* Pre-indexed filter (0 repeated querySelectorAll on keystroke) */
    const stories=[...scope.querySelectorAll('article.story')];
    const fbar=scope.querySelector('.filter-bar');
    if(fbar){
      const finp=fbar.querySelector('.filter-input');
      const fclr=fbar.querySelector('.filter-clear');
      const fcnt=fbar.querySelector('.filter-count');
      const femp=scope.querySelector('.filter-empty');
      const ftbtns=[...fbar.querySelectorAll('.filter-tag')];
      const front=scope.querySelector('section.front');
      const storyData=stories.map(el=>({
        el:el,
        hay:(el.textContent||'').toLowerCase(),
        tgs:(el.dataset.tags||'').split('|').filter(Boolean)
      }));
      const secGroups=[...scope.querySelectorAll('section.sec')].map(sec=>{
        const subs=[...sec.querySelectorAll('h3.subhead')].map(sub=>{
          const list=[];let sib=sub.nextElementSibling;
          while(sib&&!sib.matches('h3.subhead')){
            if(sib.matches('article.story'))list.push(sib);
            sib=sib.nextElementSibling;
          }
          return {sub:sub,list:list};
        });
        return {sec:sec,list:[...sec.querySelectorAll('article.story')],subs:subs};
      });
      const tpl=fbar.dataset.tpl||'{shown} / {total}';
      let curTag='',filterRaf=0;
      function syncTags(){
        ftbtns.forEach(b=>b.setAttribute('aria-pressed',(b.dataset.tag||'')===curTag?'true':'false'));
      }
      function runFilter(){
        filterRaf=0;
        const q=(finp.value||'').trim().toLowerCase();
        const toks=q?q.split(/\\s+/):[];
        const active=Boolean(toks.length||curTag);
        let shown=0;
        for(let i=0;i<storyData.length;i++){
          const sd=storyData[i];
          const ok=(!curTag||sd.tgs.includes(curTag))&&toks.every(k=>sd.hay.includes(k));
          sd.el.hidden=!ok;
          if(ok)shown++;
        }
        for(let i=0;i<secGroups.length;i++){
          const g=secGroups[i];
          let secHas=false;
          for(let j=0;j<g.list.length;j++){if(!g.list[j].hidden){secHas=true;break;}}
          g.sec.hidden=active&&!secHas;
          for(let j=0;j<g.subs.length;j++){
            const sg=g.subs[j];let subHas=false;
            for(let m=0;m<sg.list.length;m++){if(!sg.list[m].hidden){subHas=true;break;}}
            sg.sub.hidden=active&&!subHas;
          }
        }
        if(front)front.hidden=active;
        fbar.classList.toggle('is-active',active);
        fclr.hidden=!active;
        fcnt.hidden=!active;
        if(active)fcnt.textContent=tpl.replace('{shown}',String(shown)).replace('{total}',String(stories.length));
        if(femp)femp.hidden=!(active&&shown===0);
        tocLockUntil=0;
        rebuildTargets();
        syncToc(scrollY);
      }
      function schedFilter(){if(!filterRaf)filterRaf=requestAnimationFrame(runFilter);}
      function resetFilter(){finp.value='';curTag='';syncTags();runFilter();}
      finp.addEventListener('input',schedFilter);
      finp.addEventListener('keydown',e=>{if(e.key==='Escape')resetFilter();});
      fclr.addEventListener('click',()=>{resetFilter();finp.focus();});
      ftbtns.forEach(b=>b.addEventListener('click',()=>{
        const tg=b.dataset.tag||'';
        curTag=(tg&&curTag===tg)?'':tg;
        syncTags();runFilter();
      }));
      scope.addEventListener('click',e=>{
        const badge=e.target.closest('.story-meta .badge[data-tag]');
        if(badge){
          const tg=badge.dataset.tag||'';
          curTag=(curTag===tg)?'':tg;
          syncTags();runFilter();
          return;
        }
        const anc=e.target.closest('a[href^="#"]');
        if(anc&&(finp.value||curTag)){
          const tid=decodeURIComponent(anc.getAttribute('href').slice(1));
          const tel=document.getElementById(tid);
          if(tel&&tel.closest('[hidden]'))resetFilter();
        }
      },true);
      addEventListener('keydown',e=>{
        if(e.key==='/'&&isVisible(scope)&&!e.metaKey&&!e.ctrlKey&&!e.altKey&&!/^(INPUT|TEXTAREA|SELECT)$/.test((e.target&&e.target.tagName)||'')){
          e.preventDefault();finp.focus();
        }
      });
    }

    /* Delegated story copy-permalink button */
    function fallbackCopy(txt){
      try{
        const ta=document.createElement('textarea');
        ta.value=txt;ta.style.cssText='position:fixed;opacity:0';
        document.body.appendChild(ta);ta.select();
        const ok=document.execCommand('copy');ta.remove();
        return Boolean(ok);
      }catch(_){return false;}
    }
    scope.addEventListener('click',e=>{
      const btn=e.target.closest('.copy-link[data-copy-anchor]');
      if(!btn)return;
      const id=btn.dataset.copyAnchor;
      const url=location.href.split('#')[0]+'#'+id;
      try{history.replaceState(null,'','#'+id);}catch(_){}
      const done=()=>{
        btn.classList.add('is-copied');
        clearTimeout(btn._ct);
        btn._ct=setTimeout(()=>btn.classList.remove('is-copied'),1600);
      };
      if(navigator.clipboard&&navigator.clipboard.writeText){
        navigator.clipboard.writeText(url).then(done).catch(()=>{if(fallbackCopy(url))done();});
      }else if(fallbackCopy(url)){
        done();
      }
    });

    return {
      scope:scope,mast:mast,stories:stories,
      syncChrome:syncChrome,syncMastH:syncMastH,rebuildTargets:rebuildTargets,reset:reset,close:close,
      pinned:()=>mastHover||mastFocus||document.body.classList.contains('toc-open')
    };
  });

  /* Single scroll loop: hide on scroll down, reveal immediately on scroll up or after 3s idle */
  function onScrollFrame(){
    sRaf=0;
    const y=Math.max(0,scrollY),dy=y-lastY;
    lastY=y;
    setups.forEach(s=>s.syncChrome(y));
    if(y<=64){
      accDown=0;accUp=0;
      setBarHidden(false);
      return;
    }
    if(dy>0){
      accDown+=dy;accUp=0;
      if(accDown>=14){setBarHidden(true);accDown=0;}
      if(barHidden)armIdleReveal();
    }else if(dy<0){
      accUp+=-dy;accDown=0;
      if(accUp>=12){setBarHidden(false);accUp=0;}
      else if(barHidden)armIdleReveal();
    }else if(barHidden){
      armIdleReveal();
    }
  }
  addEventListener('scroll',()=>{if(!sRaf)sRaf=requestAnimationFrame(onScrollFrame);},{passive:true});
  addEventListener('resize',()=>{setups.forEach(s=>{s.syncMastH();s.syncChrome(scrollY);});},{passive:true});
  addEventListener('pointermove',e=>{if(e.clientY<=12)setBarHidden(false);},{passive:true});

  /* Single merged Language toggle button (bilingual pages) */
  let toggleLang=null;
  if(panes.length){
    const de=document.documentElement;
    const LK='digest-lang';
    const lbtns=[...document.querySelectorAll('[data-lang-toggle]')];
    const enPane=panes.find(p=>p.dataset.lang==='en');
    const titles={zh:document.title,en:(enPane&&enPane.dataset.title)||document.title};
    const curLang=()=>de.dataset.lang==='en'?'en':'zh';
    const paintLang=(animate)=>{
      const l=curLang();
      lbtns.forEach(b=>{
        b.dataset.activeLang=l;
        b.setAttribute('aria-pressed',l==='en'?'true':'false');
        if(animate)pop(b);
      });
    };
    function anchorOf(pane){
      let r=null;
      for(const e of pane.querySelectorAll('article.story, section.sec')){
        if(e.hidden)continue;
        if(e.getBoundingClientRect().top<=110)r=e;else break;
      }
      return r;
    }
    function setLang(l){
      const from=curLang();
      if(l===from){paintLang(false);return;}
      const fp=panes.find(p=>p.dataset.lang===from);
      const ref=fp?anchorOf(fp):null;
      const off=ref?ref.getBoundingClientRect().top:0;
      const key=ref?ref.id.replace(/^en-/,''):null;
      de.dataset.lang=l;de.lang=(l==='en'?'en':'zh-CN');document.title=titles[l];
      try{localStorage.setItem(LK,l);}catch(e){}
      setups.forEach(s=>{s.close();s.reset();});
      paintLang(true);
      if(key){
        const tp=document.getElementById((l==='en'?'en-':'')+key);
        if(tp&&!tp.closest('[hidden]'))scrollTo({top:scrollY+tp.getBoundingClientRect().top-off,behavior:'instant'});
      }else if(scrollY<60){scrollTo({top:0,behavior:'instant'});}
      lastY=Math.max(0,scrollY);accDown=0;accUp=0;setBarHidden(false);
      setups.forEach(s=>{s.syncMastH();s.rebuildTargets();s.syncChrome(scrollY);});
    }
    toggleLang=()=>setLang(curLang()==='en'?'zh':'en');
    lbtns.forEach(b=>b.addEventListener('click',toggleLang));
    document.title=titles[curLang()];paintLang(false);
  }

  /* Reader keyboard shortcuts: j/k stories, t theme, l lang, [/] prev/next issue */
  let kbdEl=null;
  addEventListener('keydown',e=>{
    if(e.metaKey||e.ctrlKey||e.altKey)return;
    if(/^(INPUT|TEXTAREA|SELECT)$/.test((e.target&&e.target.tagName)||''))return;
    const k=e.key;
    if(k==='ArrowDown'||k==='ArrowUp'||k==='PageDown'||k==='PageUp'||k==='Home'||k==='End'||k===' '||k==='j'||k==='k')tocLockUntil=0;
    if(k==='t'||k==='T'){e.preventDefault();cycleTheme();setBarHidden(false);return;}
    if(k==='l'||k==='L'){
      if(toggleLang){e.preventDefault();toggleLang();return;}
      const sc=activeScope();
      const a=sc&&sc.querySelector('a.lang-btn[href]');
      if(a&&a.href){e.preventDefault();location.href=a.href;return;}
    }
    if(k==='j'||k==='k'){
      const s=setups.find(x=>isVisible(x.scope));
      if(!s)return;
      const vis=s.stories.filter(el=>!el.hidden&&!(el.closest&&el.closest('[hidden]')));
      if(!vis.length)return;
      const probe=(s.mast?s.mast.offsetHeight:58)+40;
      let idx=-1;
      for(let i=0;i<vis.length;i++){
        if(vis[i].getBoundingClientRect().top<=probe)idx=i;else break;
      }
      const nextIdx=k==='j'?Math.min(vis.length-1,idx+1):Math.max(0,idx<=0?0:idx-1);
      const target=vis[nextIdx];
      if(target){
        e.preventDefault();
        if(kbdEl)kbdEl.classList.remove('kbd-focus');
        kbdEl=target;target.classList.add('kbd-focus');
        target.scrollIntoView({behavior:'smooth',block:'start'});
        setTimeout(()=>target.classList.remove('kbd-focus'),900);
      }
      return;
    }
    if(k==='['||k===']'){
      const sc=activeScope();
      const nav=sc&&sc.querySelector('.issue-nav.top');
      if(!nav)return;
      const a=nav.querySelector(k==='['?'a[data-nav="prev"]':'a[data-nav="next"]');
      if(a&&a.href)location.href=a.href;
    }
  });

  setups.forEach(s=>{s.syncMastH();s.rebuildTargets();s.syncChrome(scrollY);});
  if(location.hash.length>1){
    let id=location.hash.slice(1);try{id=decodeURIComponent(id);}catch(e){}
    let el=document.getElementById(id);
    if(panes.length){
      const isEn=document.documentElement.dataset.lang==='en';
      const base=id.replace(/^en-/,'');
      const want=(isEn?'en-':'')+base;
      el=document.getElementById(want)||el;
    }
    if(el)requestAnimationFrame(()=>el.scrollIntoView());
  }
})();
</script>"""


def _estimate_read_minutes(digest: Digest, lang: str) -> int:
    cjk = latin = 0
    for it in digest.items:
        text = " ".join([it.title, *it.fields.values(), it.extra])
        cjk += sum(1 for ch in text if "\u4e00" <= ch <= "\u9fff")
        latin += len(re.findall(r"[A-Za-z0-9]+", text))
    mins = (cjk / 380.0) + (latin / 220.0)
    return max(1, int(round(mins)))


def _theme_toggle_html(lang: str) -> str:
    lbl_sys = i18n.t(lang, "theme_system")
    lbl_light = i18n.t(lang, "theme_light")
    lbl_dark = i18n.t(lang, "theme_dark")
    labels_json = html.escape(
        f'{{"system":"{lbl_sys}","light":"{lbl_light}","dark":"{lbl_dark}"}}',
        quote=True,
    )
    aria_base = html.escape(i18n.t(lang, "appearance"), quote=True)
    title = html.escape(i18n.t(lang, "theme_toggle_title"), quote=True)
    return (
        f'<button type="button" class="ctrl-btn theme-btn theme-switch" data-theme-toggle data-mode="system"'
        f' data-aria-base="{aria_base}" data-labels="{labels_json}" aria-label="{aria_base}" title="{title}">'
        '<span class="theme-ics" aria-hidden="true">'
        '<svg class="ic-sys" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="5.8"/><path d="M8 2.2A5.8 5.8 0 0 1 8 13.8Z" fill="currentColor" stroke="none"/></svg>'
        '<svg class="ic-sun" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><circle cx="8" cy="8" r="3"/><path d="M8 1.6v1.6M8 12.8v1.6M1.6 8h1.6M12.8 8h1.6M3.5 3.5l1.1 1.1M11.4 11.4l1.1 1.1M12.5 3.5l-1.1 1.1M4.6 11.4l-1.1 1.1"/></svg>'
        '<svg class="ic-moon" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M13.2 9.8A5.5 5.5 0 0 1 6.2 2.8a5.6 5.6 0 1 0 7 7Z"/></svg>'
        '</span>'
        f'<span class="ctrl-lbl">{html.escape(lbl_sys)}</span>'
        "</button>"
    )


def _lang_toggle_html(lang: str) -> str:
    aria = html.escape(i18n.t(lang, "aria_lang_switch"), quote=True)
    title = html.escape(i18n.t(lang, "lang_toggle_title"), quote=True)
    zh_code = html.escape(i18n.t("zh-CN", "lang_code"))
    en_code = html.escape(i18n.t("en", "lang_code"))
    return (
        f'<button type="button" class="ctrl-btn lang-btn lang-switch" data-lang-toggle data-active-lang="zh"'
        f' aria-label="{aria}" title="{title}">'
        '<span class="ctrl-ic" aria-hidden="true">'
        '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">'
        '<circle cx="8" cy="8" r="6.2"/>'
        '<path d="M1.8 8h12.4M8 1.8c1.9 1.8 2.8 3.9 2.8 6.2S9.9 12.4 8 14.2C6.1 12.4 5.2 10.3 5.2 8S6.1 3.6 8 1.8Z"/>'
        "</svg></span>"
        '<span class="slot-roll" aria-hidden="true"><span class="slot-track">'
        f'<span class="slot-v">{zh_code}</span><span class="slot-v">{en_code}</span>'
        "</span></span></button>"
    )


def _render_pane_body(
    digest: Digest,
    lang: str,
    *,
    year: int | None = None,
    id_prefix: str = "",
    lang_switch: bool = False,
) -> tuple[str, list[str]]:
    lang = i18n.normalize_lang(lang)
    weekly = i18n.looks_weekly(digest.title)
    info = _period_info(digest.period, weekly, lang, year=year)
    read_mins = _estimate_read_minutes(digest, lang)
    read_str = i18n.t(lang, "read_time", m=read_mins)
    full_title = digest.title
    if digest.period:
        full_title = f"{digest.title} · {digest.period}"
    front_left = render_themes(digest, lang, id_prefix=id_prefix)
    sidebar = render_sidebar(digest, lang, id_prefix=id_prefix)
    issue_tags_list: list[str] = []
    seen_tags: set[str] = set()
    for it in digest.items:
        for tg in _split_meta(it.meta)[2]:
            if tg and tg not in seen_tags:
                seen_tags.add(tg)
                issue_tags_list.append(tg)
    tag_chips = ""
    if issue_tags_list:
        chips = [
            f'<button type="button" class="filter-tag" data-tag="" aria-pressed="true">{html.escape(i18n.t(lang, "filter_all"))}</button>'
        ] + [
            f'<button type="button" class="filter-tag" data-tag="{html.escape(tg, quote=True)}" aria-pressed="false">{html.escape(tg)}</button>'
            for tg in issue_tags_list
        ]
        tag_chips = '<div class="filter-tags" role="group">' + "".join(chips) + "</div>"
    filter_bar_html = (
        f'<div class="filter-bar" id="{id_prefix}digestFilterBar" role="search" aria-label="{html.escape(i18n.t(lang, "filter_label"), quote=True)}"'
        f' data-total="{len(digest.items)}" data-tpl="{html.escape(i18n.t(lang, "filter_shown"), quote=True)}">'
        '<div class="filter-row">'
        f'<input type="search" id="{id_prefix}digestFilterInput" class="filter-input" placeholder="{html.escape(i18n.t(lang, "filter_placeholder"), quote=True)}"'
        f' aria-label="{html.escape(i18n.t(lang, "filter_label"), quote=True)}" autocomplete="off" spellcheck="false">'
        '<kbd class="filter-kbd" aria-hidden="true">/</kbd>'
        f'<span id="{id_prefix}digestFilterCount" class="filter-count num" aria-live="polite" hidden></span>'
        f'<button type="button" id="{id_prefix}digestFilterClear" class="filter-clear" hidden>{html.escape(i18n.t(lang, "filter_clear"))}</button>'
        f"</div>{tag_chips}</div>"
        f'<p id="{id_prefix}digestFilterEmpty" class="filter-empty" hidden>{html.escape(i18n.t(lang, "filter_empty"))}</p>'
    )
    parts = [
        f'<a class="skip" href="#{id_prefix}content">{html.escape(i18n.t(lang, "skip"))}</a>',
        '<header class="mast">',
        '<div class="mast-bar">',
        f'<h1 id="{id_prefix}report-start" class="np">{_nameplate_html(digest.title)}</h1>',
        f'<span class="mast-chip"><span class="mark-kicker">{html.escape(info["kicker"])}</span><span class="mark-figure num">{html.escape(info["figure"])}</span></span>',
        f'<span class="mast-meta"><span class="num">{html.escape(info["line"])}</span><span class="num">{html.escape(i18n.count_items(lang, len(digest.items)))} · {html.escape(read_str)}</span><span class="motto">{html.escape(info["motto"])}</span></span>',
        '<span class="mast-tools">',
        f'<span class="edition">{html.escape(info["edition"])}</span>',
    ]
    if lang_switch:
        parts.append(_lang_toggle_html(lang))
    parts.extend([
        _theme_toggle_html(lang),
        "</span>",
        "</div>",
        '<div class="read-prog" aria-hidden="true"><span class="read-prog-fill"></span></div>',
        "</header>",
        '<article class="paper">',
        '<div class="layout">',
        sidebar,
        '<div class="layout-body">',
    ])
    if digest.must or digest.themes:
        # Weekly 头版: stacked bands, then straight into the sections.
        must_html = render_must(digest, lang, id_prefix=id_prefix)
        parts.append(f'<section class="front front-stack" aria-label="{html.escape(i18n.t(lang, "front_aria"))}">')
        if must_html:
            parts.append(f'<div class="front-must">{must_html}</div>')
        if front_left:
            parts.append(f'<div class="front-lead">{front_left}</div>')
        parts.append("</section>")
    parts.append(filter_bar_html)
    parts.append(f'<main id="{id_prefix}content">')
    for i, sec in enumerate(digest.sections, 1):
        sid = id_prefix + _section_id(sec.title)
        label = _plain_section(sec.title)
        parts.append(f'<section class="sec" id="{html.escape(sid)}">')
        parts.append(
            f'<header class="sec-head"><span class="sec-no num">{i:02d}</span>'
            f"<h2>{html.escape(label)}</h2></header>"
        )
        if sec.note:
            parts.append(f'<p class="sec-note">{html.escape(sec.note)}</p>')
        for block in sec.blocks:
            if isinstance(block, Item):
                parts.append(render_item(block, lang, id_prefix=id_prefix))
            elif isinstance(block, tuple) and block[0] == "sub":
                parts.append(
                    f'<h3 class="subhead"><span class="bar"></span>{html.escape(block[1])}</h3>'
                )
        parts.append("</section>")
    parts.append("</main>")
    parts.append(
        f'<footer class="colophon"><div class="end">{html.escape(i18n.t(lang, "end"))}</div></footer>'
    )
    parts.append("</div></div>")  # .layout-body, .layout
    parts.append("</article>")
    parts.append(
        f'<button class="to-top" id="{id_prefix}toTop" type="button" aria-label="{html.escape(i18n.t(lang, "to_top"))}">↑</button>'
    )
    return full_title, parts


def _html_head(title: str, html_lang: str, *, bilingual: bool = False) -> list[str]:
    parts = [
        "<!DOCTYPE html>",
        f'<html lang="{html_lang}">',
        "<head>",
        '<meta charset="utf-8">',
        '<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">',
        '<meta name="theme-color" content="#faf8f4">',
        f"<title>{html.escape(title)}</title>",
        "<script>(function(){try{var t=localStorage.getItem('digest-theme');if(t==='light'||t==='dark')document.documentElement.setAttribute('data-theme',t);var dk=t==='dark'||(t!=='light'&&window.matchMedia&&matchMedia('(prefers-color-scheme: dark)').matches);var m=document.querySelector('meta[name=\"theme-color\"]');if(m)m.setAttribute('content',dk?'#141311':'#faf8f4');}catch(e){}})();</script>",
    ]
    if bilingual:
        parts.append(
            "<script>(function(){try{var l=localStorage.getItem('digest-lang');if(l==='en'){document.documentElement.dataset.lang='en';document.documentElement.lang='en';}}catch(e){}})();</script>"
        )
    parts.extend([
        "<style>",
        _load_css(),
        "</style>",
        "</head>",
        "<body>",
    ])
    return parts


def render_html(digest: Digest, lang: str | None = None, year: int | None = None) -> str:
    lang = i18n.resolve_lang(lang)
    full_title, body_parts = _render_pane_body(digest, lang, year=year)
    parts = _html_head(full_title, i18n.t(lang, "html_lang"), bilingual=False)
    parts.extend(body_parts)
    parts.append(_FOOTER_SCRIPT)
    parts.append("</body></html>")
    return "\n".join(parts) + "\n"


def render_bilingual_html(zh_digest: Digest, en_digest: Digest, year: int | None = None) -> str:
    """Render primary zh-CN digest and English companion digest into one toggleable HTML page."""
    zh_title, zh_parts = _render_pane_body(zh_digest, "zh-CN", year=year, id_prefix="", lang_switch=True)
    en_title, en_parts = _render_pane_body(en_digest, "en", year=year, id_prefix="en-", lang_switch=True)
    parts = _html_head(zh_title, i18n.t("zh-CN", "html_lang"), bilingual=True)
    parts.append('<div class="lang-pane lang-zh" data-lang="zh">')
    parts.extend(zh_parts)
    parts.append("</div>")
    parts.append("<!-- digest-lang:en:start -->")
    parts.append(f'<div class="lang-pane lang-en" data-lang="en" lang="en" data-title="{html.escape(en_title, quote=True)}">')
    parts.extend(en_parts)
    parts.append("</div>")
    parts.append("<!-- digest-lang:en:end -->")
    parts.append(_FOOTER_SCRIPT)
    parts.append("</body></html>")
    return "\n".join(parts) + "\n"


def _section_id(title: str) -> str:
    plain = _plain_section(title).lower()
    for keys, sid in i18n.SECTION_IDS:
        if any(k in plain for k in keys):
            return sid
    slug = re.sub(r"[^\w]+", "-", title, flags=re.U).strip("-")
    return "sec-" + (slug or "section")


def render_file(
    src: Path,
    dest: Path,
    lang: str | None = None,
    year: int | None = None,
    en_md_path: Path | None = None,
) -> Digest:
    inferred_year = year if year is not None else _year_from_path(src)
    dest.parent.mkdir(parents=True, exist_ok=True)
    if en_md_path is not None:
        zh_digest = parse_digest(src.read_text(encoding="utf-8"), "zh-CN")
        en_digest = parse_digest(en_md_path.read_text(encoding="utf-8"), "en")
        if len(en_digest.items) != len(zh_digest.items):
            print(
                f"WARNING: English has {len(en_digest.items)} items vs Chinese {len(zh_digest.items)}",
                file=sys.stderr,
            )
        dest.write_text(render_bilingual_html(zh_digest, en_digest, year=inferred_year), encoding="utf-8")
        return zh_digest
    lang = i18n.resolve_lang(lang)
    digest = parse_digest(src.read_text(encoding="utf-8"), lang)
    dest.write_text(render_html(digest, lang, year=inferred_year), encoding="utf-8")
    return digest


def main(argv: list[str] | None = None) -> int:
    import argparse

    ap = argparse.ArgumentParser(
        description="Render a digest markdown file to one self-contained HTML page.",
        epilog=(
            "examples:\n"
            "  python3 render_html.py digest-2026-01-15.md html/daily-2026-01-15.html\n"
            "  python3 render_html.py examples/sample-digest.zh-CN.md /tmp/sample.html --lang zh-CN\n"
            "  python3 render_html.py digest.md out.html --md-en digest.en.md  # single-page bilingual\n"
            "  python3 render_html.py digest.md out.html --json      # one JSON object on stdout\n\n"
            "Markdown from either language parses (English or Chinese labels). Exit codes: "
            "0 ok; 1 input not found or malformed Markdown; 2 bad --lang."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--version", action="version", version=f"ai-digest {__version__}")
    ap.add_argument("input", type=Path, help="digest markdown (digest-<date>.md or weekly-<range>.md)")
    ap.add_argument("output", type=Path, help="HTML file to write (parent directories are created)")
    ap.add_argument("--lang", help="UI language: en | zh-CN (default: $DIGEST_LANG, else en)")
    ap.add_argument("--md-en", type=Path, help="English companion markdown to bundle into a single bilingual HTML page (primary input rendered as zh-CN)")
    ap.add_argument("--year", type=int, help="issue year override when heading omits it (default: inferred from filename, else current year)")
    ap.add_argument("--json", action="store_true", help="print one JSON object {output, lang, title, items, sections} instead of the text line")
    ns = ap.parse_args(sys.argv[1:] if argv is None else argv)
    try:
        lang = i18n.resolve_lang(ns.lang)
    except ValueError as e:
        print(str(e), file=sys.stderr)
        return 2
    src, dest = ns.input, ns.output
    if not src.exists():
        print(f"input not found: {src}", file=sys.stderr)
        return 1
    if ns.md_en is not None and not ns.md_en.exists():
        print(f"English input not found: {ns.md_en}", file=sys.stderr)
        return 1
    try:
        digest = render_file(src, dest, lang, year=ns.year, en_md_path=ns.md_en)
    except ValueError as e:
        print(f"error: {src}: {e}", file=sys.stderr)
        return 1
    out_lang = "zh-CN" if ns.md_en is not None else lang
    if ns.json:
        import json
        payload = {
            "output": str(dest),
            "lang": out_lang,
            "title": digest.title,
            "items": len(digest.items),
            "sections": len(digest.sections),
        }
        if ns.md_en is not None:
            payload["bilingual"] = True
        print(json.dumps(payload, ensure_ascii=False))
    else:
        suffix = ", bilingual" if ns.md_en is not None else ""
        print(f"wrote {dest} ({len(digest.items)} items{suffix})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
