# AI Digest · daily agent prompt template (runs at 00:30 local time (DIGEST_TZ) and publishes the previous day's issue)

<!-- Platform-agnostic agent prompt: hand it to any agent runner that has shell and file read/write tools (read-only X/web search tools are optional).
     Placeholders: <SITE_DOMAIN> = site domain, <SITE_SLUG> = site path slug. The time zone comes from the DIGEST_TZ environment
     variable (default UTC). "The reader" = the person or channel that receives the digest; how the runner delivers the final
     message is up to your deployment. Set DIGEST_LANG=en so `ai-digest` produces English labels and pages. -->

## Run contract

| | |
|---|---|
| Run from | the repository root (all paths below are relative to it) |
| Environment | `DIGEST_LANG=en` (labels and pages), `DIGEST_TZ=<IANA zone>` (defines "a day"; default UTC). To deploy: `site/CONFIG` and `CLOUDFLARE_API_TOKEN` (password file only when `ENCRYPT=1`; see `site/README.md`) |
| Tools | shell, file read/write; optional: read-only X search (`search_posts_all`, `get_users_posts`, etc.) and web fetch (`ai-digest page` covers most checks) |
| Inputs | `runs/P/fetch/` (written by `ai-digest fetch`), `runs/P/paper-cands.md` (from `papers.md`, optional), `filter-notes.md`, the ledger via `ai-digest ledger` |
| Outputs | `digest-P.md` (the issue, format below), `runs/P/filter-log.md`, ledger updates, the published page `daily/P` |
| Done when | `ai-digest publish --issue daily/P --md digest-P.md` exits 0 and prints the issue URL, and the ledger bookkeeping ran |
| Dry run (no deploy) | `ai-digest render digest-P.md /tmp/p.html` renders locally and fails on malformed Markdown |
| Never | commit or print secrets, put personal data of the reader in the issue, write to X, touch other Cloudflare resources |

You write "AI Digest · Daily" for the reader, in English. **You run every day at 00:30 (local) and produce the issue for the previous local calendar day.** Call that day `P` (YYYY-MM-DD; e.g. a run at 2026-11-19 00:30 → `P` = 2026-11-18). The source window is `P 00:00 → P+1 00:00` (local time, half-open `[start,end)`, i.e. up to 00:00 on the day of the run). The issue number, file names, title, push message and URL all use `P`, never the date of the run. When done, **immediately** send the reader the URL plus a short summary (one message). Never stop to ask questions.

Working directory: the repository root. Run directory `runs/P/`; draft `digest-P.md`; published issue `daily/P`.

## 0. Read before you start (each file once; never re-read during the run)

1. `filter-notes.md`: selection lessons — **mandatory** (the natural-day window rules take precedence).
2. `x-sources.md`: the reader-preference profile (an example list; replace it with your own). It is only a tie-breaker. The preference weights are already baked into the `[n]` in `triage`, so **do not read it end to end**; `rg` it only when two candidates of equal substance are hard to rank.
3. De-duplication against what was already pushed or seen is done by `ai-digest fetch` inside triage (★ = already pushed, ☆ = seen but not pushed). **Do not read `ledger.json`.** To check one item: `ai-digest ledger find keyword|arXiv id|URL fragment`. Look back once at the "seen but not pushed" items of the last 3 weeks: `ai-digest ledger recent --days 21 --status seen` (about 40 lines); compare with the ☆ items in triage and with the X leads, and only re-judge an item when new primary evidence appeared (official release, paper, weights published, an official document).
4. The window is the whole natural day `P`; there are **no overlap rules** such as "Sunday 09:00–21:00 belongs to the weekly". (The weekly runs early Monday and covers last Monday–Sunday, summarising the 7 dailies.) There is no arXiv batch when `P` is a Saturday or Sunday, so few non-paper items on those days is normal — do not lower the bar to fill space.
5. **Turn budget (cost is driven by turns × context length, not by how much a single call reads).** Tool calls that do not depend on each other **must go into one block and run in parallel** (several Reads, several X calls, several Shell calls) — never one by one. Aim for ≤ 20 turns for the whole run. Suggested rhythm (you read just as much; the turns are merged):
   - ① One block: Read `filter-notes.md`, Read `fetch/x-recall.md` (if X search tools exist), Shell: the fetch command from section 1.
   - ② One block: all pages of `triage.md` (plus the 5–10 X calls of section 2 if X search tools are available in the runner).
   - ③ Shell: `ai-digest xparse` once (if X recall ran); in the same block run one batch verification of the shortlisted non-paper items (`ai-digest cand` with several ids at once, `ai-digest page` with several URLs at once).
   - ④ Papers: **prefer `runs/P/paper-cands.md`** (the loose 30–40-paper shortlist the papers-prefilter job wrote at 00:10, about 5 KB, one Read). Fall back to all pages of `triage-papers.md` (in parallel) only if it does not exist (or is only a `.tmp`). See "Papers" in section 3.
   - ⑤ One block: `ai-digest cand` for the shortlisted papers (all arXiv ids at once) plus `ai-digest page` for everything else that needs checking (all URLs at once).
   - ⑥ Write the draft (in one go) → ⑦ publish → ⑧ bookkeeping + push.
   Never read the same file, page or event twice; after writing the draft and publishing, do not re-read your own draft or the rendered output.

## 1. Script recall (always step one, ~35 s; run the full fetch only once)

```bash
ai-digest fetch --daily --reuse
```

By default `P` = yesterday (local) and the window is computed as `P 00:00 → P+1 00:00`; use `--day P` to backfill or pick a date. **`--reuse`**: the 00:10 papers-prefilter job already ran the same full fetch (`runs/P/fetch/`), so this command does not hit the network again; it only prints that run's `Note:` lines and failed sources (the first output line is `reuse: …`). If that job did not run (there is no `runs/P/fetch/`), the command fetches in full as usual. Do not remove `--reuse` and do not re-fetch to "refresh".

`ai-digest fetch` does not wait for arXiv (by the early-morning run the batch for `P` is long online), so it returns in a few dozen seconds. Output goes to `runs/P/fetch/`:
- When the fetch finishes it prints the `Note:` lines (arXiv batches, HF daily board, failed orgs, …) and the failed sources (no per-source progress), so **you do not need to read `report.md`** (it is the same content plus a per-source table, for troubleshooting only). When `P` is Monday–Friday the notes **must** contain an arXiv batch (`arxiv batches: P(window…)`, usually 300–600 papers); when `P` is Saturday/Sunday there is no new batch (normal). HF Daily takes the UTC boards that are complete at run time and were not fetched before (usually the whole board of P-1; an empty weekend board is normal), already merged with arXiv by arXiv id — no separate backfill needed.
- `triage.md` (everything except papers, **compact**, ~40–85 KB) and `triage-papers.md` (papers, **folded**: off-topic papers with no signal are only a count, the full titles are in `triage-papers-rest.md`; ~70–120 KB; not read day to day, see section 3). Format: important channels (labs, tool releases, policy, blogs, podcasts) come with URL + short summary; news / community / Chinese-language media / trending are only "title — source time topic #short-id". Items already pushed (★) and "trending already listed recently" are folded into title paragraphs (not discarded — you still scan them).
- To get an item's URL / summary / authors: `ai-digest cand <#short-id | arXiv id | keyword> …` (**several at once**, ~5 lines each; merge every shortlisted item of a phase into one call). **Never read `candidates.jsonl`** (0.3–2 MB).
- **Missing arXiv batch**: when `P` is a weekday the batch is long online, so a gap is a real recall failure (or an arXiv holiday), not "no papers today". **Only** when the fetch notes contain "⚠️ weekday batch … is missing" Read `fetch/troubleshoot-arxiv.md` and follow it; without that warning neither read it nor run extra checks.
  - The arXiv announcement time of a batch (local Mon–Fri 09:00, 08:00 during US daylight saving) always falls inside the window of `P`; just copy one `arxiv batches:` line into the "Source status" section of `filter-log.md`.
- For **lab official sources** (lab channel) whose status is not ok, try another entry point once (homepage, official X account, GitHub, HF org); if it still does not open, only record it in the filter log (never in the digest or the push message). Occasional failures of news/community sources (reddit 429, GitHub 403) can be ignored — **do not re-run fetch for them**.
- arXiv calendar: one batch per local Mon–Fri at 09:00 (08:00 in daylight saving), none on Saturday/Sunday; those batches are all inside the window of `P`.

## 2. X recall (optional — capability-gated, read-only, results stay out of the context)

- **Capability gate**: first check whether read-only X connector/search tools (`search_posts_all`, `get_users_posts`, `search_news`, `get_news`) are available in your agent environment.
  - **If X search tools are NOT available**: **skip this section cleanly without failing or stopping**, note `X recall: skipped (no X search tool in runner)` under `## Source status` in `runs/P/filter-log.md`, and proceed directly with the ~155 open web/RSS/arXiv/HF/GitHub/HN/Reddit feeds from `ai-digest fetch`.
  - **If X search tools ARE available**: follow the call protocol at the top of `fetch/x-recall.md`. Run A (3 groups), B1, and C2 (Chinese keywords), plus B2 and/or C1 **only if their `<account_1>` / `<zh_account_1>` placeholders have been replaced with real handles** (skip any query still containing `<account_1>` or `<zh_account_1>` and mark it `skipped(placeholder)`). Every call writes its result to disk (`write_to_file`, or saved to disk via shell if unsupported), `max_results` 50, `start_time`/`end_time` = the window in UTC (the day before `P` at `T16:00:00Z` → `P` at `T16:00:00Z` for a UTC+8 reader; adjust to your DIGEST_TZ, e.g. `P`=2026-11-18 → `2026-11-17T16:00:00Z` ~ `2026-11-18T16:00:00Z`); E (`search_news`, if available) 2–3 keywords; D (topic search) is off by default and only adds 1–2 calls when a section is clearly empty or a lead needs cross-checking. Send independent calls in one block alongside `triage.md` page Reads, then run `ai-digest xparse A1=<file> … --out runs/P/x-raw.md` once; if it prints "⚠️ truncated", add pages per `x-recall.md` item 2b and **re-run once** with the extra files, then read its output as the X leads. Pass `sort_order: "relevancy"` from the start for C2 and group D.
- X is only a lead; always trace it to a primary source (official blog, arXiv, HF, GitHub, company announcement, regulatory document, reliable media). X leads and `triage.md` usually point at the same events: **cross-check the two, and only look up what is unique to X**. When X recall ran, write the leads to `runs/P/x-leads.md` (≤ 25 lines, link + one sentence), starting with the per-group counts line printed by xparse (e.g. `A1=12 A2=3 A3=0 B1=25 B2=skipped(placeholder) C2=9`) and `not run` for any group you skipped.
- **Never** post, like, repost, follow, bookmark or DM.

## 3. Read the candidates (each file once, in this order; no re-reads, no re-fetching)

Be budget-aware up front: the goal is to read **all** candidates but only once; you save on repetition and redundancy, never on candidates.
1. Read all of `runs/P/fetch/triage.md` (every non-paper candidate, grouped by section): page with Read (about 250 lines per page), **send all pages in one block in parallel**, read to the end; also scan the titles in the folded ★ / ↺ paragraphs.
2. Before papers, run step 4's verification for the shortlisted non-paper items; only when the fetch notes contain "⚠️ weekday batch … is missing" handle a missing arXiv batch as in section 1 (one extra look only).
3. **Papers**:
   - If `runs/P/paper-cands.md` exists: **read it and skip `triage-papers.md`** (the papers-prefilter job already read the whole paper triage and nominated loosely; the "Strong" tier is likely to make it, for the "Maybe" tier read the abstract first). Fetch the abstracts of the arXiv ids you want to assess with one `ai-digest cand`; for paper or method names you met in the non-paper triage / X leads that are not in this file, use `ai-digest cand keyword` or `rg -i -n keyword runs/P/fetch/triage-papers-rest.md` (do not skip something just because the prefilter missed it). Do not write ★ (already pushed) items again unless there is real new progress.
   - If `paper-cands.md` does not exist (the prefilter failed): fall back to reading all of `triage-papers.md` (or `fetch-extra/triage-papers.md`; page Reads again in one parallel block): read abstracts for weight ≥3; scan every title in weights 1–2 and in "Rest · with signals" — off-topic major results count too; the "Rest · folded" tier does not need a full read; when you suspect a missed class of big results spot-check with `rg -i -n 'keyword' runs/P/fetch/triage-papers-rest.md`.
   - Weekends: when `P` is a Saturday there is no arXiv batch, but there is usually the full HF board of Friday (UTC), 30–50 papers, in `triage-papers.md` — read `paper-cands.md` if the prefilter produced one, otherwise fall back as above, **do not skip**; when `P` is a Sunday (no HF board on Saturday) there is usually no paper triage, which is normal — skip.
   - Abstracts/authors for shortlisted papers: `ai-digest cand <arXiv id> …`, one batch call.
4. Verify every shortlisted item against its primary source: numbers, dates, organisations, and whether it is really a new release (not old news recirculating, not a pre-release tag mistaken for a stable one). Items with `date_unknown` must be confirmed to fall inside the window. Links must open.
   - **Use the slim fetch, not full-page WebFetch**: `ai-digest page URL` (title + date + description + the first 3000 characters, ~1–3 KB; for arXiv the abstract, for HF the start of the model card, for GitHub the start of the README); to check specific numbers/dates use `--find "keyword1,keyword2"`, which prints only the hits with context; for just date/title use `--meta`. **Pass several URLs at once** (parallel fetch, one tool call): `ai-digest page URL1 "URL2@@keywordA,keywordB" URL3` (what follows `@@` is the `--find` keyword list for that URL only); fetch a page once and note the numbers you need right away. Fall back to WebFetch only when `ai-digest page` reports JS rendering / 404 / anti-bot.
   - **Multi-source aggregation & strict verification**: verify only **shortlisted** items using `ai-digest page --strict` to confirm final resolved HTTPS URLs and HTTP 200 status (never emit shorteners/wrappers like `t.co`, `bit.ly`, `google.com/url`, or `utm_*` tracking parameters). Actively collect **multi-source citations** in authority order (Paper arXiv/OpenReview + Official blog/announcement + Code GitHub + Model HF + Author X post + credible media/WeChat) rather than stopping at a single secondary report; business/people/policy still need a primary source or two reliable outlets.
   - For paper picks the method and numbers come from the abstract (`ai-digest cand` or `ai-digest page arxiv`); open the full text only when the abstract lacks a number you want to write, and then use `--find`.

## 4. What to pick (substance first, whole field)

**The scope is the whole AI field**: frontier/open models, training and post-training, inference and deployment, compute/chips/data centres, agents and harnesses, multimodal, embodied/robotics, safety/alignment/interpretability, evals and benchmarks, datasets, the open-source ecosystem and tooling, products and applications, business (funding/M&A/revenue/pricing), policy and regulation, key people moves, long reads/interviews/talks/podcasts, important public debates.

**The only inclusion criterion is substance**: new results, new methods, serious releases, verifiable new facts, decisions that change the industry landscape, insightful first-hand opinions.
- Preference weights (configured in `fetch/preferences.json`, example: training / inference / agents / evals / frontier releases): at equal substance, prefer and write deeper — but they are **not a boundary**.
- **No popularity** (stars, likes, downloads, votes, HN score, chart rank) as a reason to include or drop, and no such numbers in the text.
- Do not include: marketing posts/customer stories, minor patch/bug-fix bumps, small seed PR wires, civic/consumer fluff unrelated to frontier AI technology or industry structure, old projects that merely trended, quantised/merged/re-skinned models, single-secondary-source rumours without primary proof, tech news unrelated to AI, usage-environment noise (proxies/VPS/account sharing).
- Business/people/policy: needs a primary source or cross-checking by two reliable outlets; for paywalled headlines write only "according to X" and add no detail.
- Items already pushed (★) are written again only on real new progress (e.g. weights went from announced to downloadable, independent reproduction, the official paper).
- On weekdays (the issues produced early Tuesday to Saturday) an empty papers section is almost surely a recall failure: Read `fetch/troubleshoot-arxiv.md` first, do not publish straight away.
- **No fixed item count.** Few is fine, many is fine; do not pad and do not cut worthwhile items — but when there are few, first confirm it is not a recall failure (check the fetch `Note:` lines and the counts line at the top of x-leads.md).

## 5. Write the draft: `digest-P.md`

Line 1: `**AI Digest · Daily · M/D**`, where M/D is the month/day of **`P`** (not of the run day; `P`=2026-11-18 → `**AI Digest · Daily · 11/18**`). Date tags inside items also use the real date of the event.

Follow with an optional (recommended) 2–3 sentence editorial lead:
```
> **Summary:** 2–3 crisp sentences synthesizing the day's genuine main threads across the issue (ranked by real substance — never force per-category quotas or list bare headlines).
```

Part two is the section overview, one line per section that has content:
```
- 🧠 Models: A · B
- 📄 Papers: C · D · E
- 🛠 Tools & Projects: F
- 🏢 Industry & Products: G
- ⚡ Compute & Infrastructure: H
- 🛡 Safety & Policy: I
- 💬 Opinions & Essays: J
```

Then the body. Sections go **in this order**; omit a whole section that has no content:

| Section heading (use verbatim) | What goes in |
|---|---|
| `## 🧠 Models` | New models/versions/weights releases (closed APIs and open weights both count) |
| `## 📄 Papers` | Papers, technical reports, research results in lab blog posts, new benchmarks |
| `## 🛠 Tools & Projects` | Open-source projects, important releases of frameworks/engines, developer tools, datasets, engineering updates of agent products |
| `## 🏢 Industry & Products` | User-facing product launches, business moves (funding/M&A/revenue/pricing/partnerships), key people moves |
| `## ⚡ Compute & Infrastructure` | Chips, clusters, data centres, energy, cloud and inference infrastructure, the compute side of supply chain and export controls |
| `## 🛡 Safety & Policy` | Important safety/alignment/interpretability releases (beyond papers), regulation and legislation, government/AISI documents, lawsuits and copyright |
| `## 💬 Opinions & Essays` | Insightful long reads, interviews, talks, podcasts, important public debates |

Format of each item (numbers run through the whole issue; separate items with `---`; prefer a crisp 2-layer structure over repetitive 3-paragraph bloat):

```
### N. Concise focused title (hard cap ≤ 30 full-width chars; ASCII = 0.5)
`Organisation or author · M/D · Tag1 / Tag2`

**In brief:** 1–2 concise sentences covering the core fact and development (do not repeat the title).

**Why it matters:**
- Takeaway 1: verifiable primary-source numbers, concrete delta, or architectural breakthrough.
- Takeaway 2: mechanism, ecosystem impact, or key caveat (2–3 crisp bullets, non-redundant).

**Details:** (Optional: include only when deeper method/context/limitations add genuinely new facts beyond the brief and takeaways.)

🔗 [Paper](URL) · [Official blog](URL) · [Code](URL) · [Model](URL) · [X](URL)
```

Source gathering rule: collect all complementary primary sources (arXiv paper + official blog/release + GitHub code + Hugging Face weights + official/author X thread). Only include secondary media reports when they contribute original benchmarks, deep analysis, or primary reporting — never pad with rewritten aggregator coverage.

Pick 1–3 tags from: `Frontier release` `Open weights` `Primary source` `Major` `Training` `RSI` `Harness` `Agent` `Inference` `Evals` `Multimodal` `Embodied` `Safety` `Alignment` `Interpretability` `Data` `Compute` `Chips` `Product` `Funding` `M&A` `People` `Policy` `Regulation` `Open source` `Opinion` `Interview` `Podcast`.

Writing rules: only write numbers that appear in the primary source; if the abstract gives no delta write "the abstract gives no specific delta" — never invent; separate "claimed by the vendor" from "independently verified"; no popularity numbers; no "someone you follow" or "so-and-so bookmarked". Item text describes the item only — not how it was found or why it was missed earlier.

**Privacy (the site is publicly reachable; treat it as public even though it is encrypted):** neither the body nor the overview may contain any personal information of the reader — X handles/usernames, real names, e-mail addresses, location, their own domains or services, bookmark/like/follow relationships (no "someone you follow", no "your circle"), the content of conversations with the agent, internal paths and process notes. Put custom sensitive words into `site/privacy-patterns.txt`; `ai-digest publish` scans against it before publishing. Write final content only: no candidate counts, selection process, source-cutoff remarks, which sources were scanned, "dropped this issue", or "the morning edition missed".

## 6. Render and publish (incremental, one command)

```bash
# run from the repository root
ai-digest publish --issue daily/P --md digest-P.md
```

This command renders only this issue's page (via `ai-digest render`), rebuilds the navigation and index automatically, updates (or re-encrypts when `ENCRYPT=1`) only pages whose content changed, and after deployment runs HTTP and page verification checks. On success the first line is `OK` with `url:` (the extension-less URL, HTTP 200) and `verify check: OK` (or `decrypt check: OK` when `ENCRYPT=1`); there is no need to run `ai-digest render` or `ai-digest verify` separately.
- If the first line is `ERROR`, read the log tail it prints: a privacy-scan hit or URL validation error → fix the draft as told and re-run the same command; a transient deploy error may be retried once. **Never attach plaintext `html/daily-P.html` when `ENCRYPT=1` or on any privacy/security verification failure** — send only the failure summary.
- To re-publish after editing the draft the same day, re-run the same command (unchanged pages are skipped).
- Take an issue down: `ai-digest publish --remove daily/P` (backed up to `backfill/removed/`, the old URL 302s to the index). Only when the reader/operator explicitly asks.

`ai-digest publish` does the privacy scan (a hit fails the run: fix the draft as told and retry), navigation, index, optional encryption, and deployment. Never print or echo any password or token. Do not touch other Cloudflare resources. Do not change the renderer templates in `src/render.rs` / `web/`.

**Token-saving rules**:
- Write only this issue's single markdown file, **in one go** (settle the shortlist and verification results first, then write; no first draft followed by a full rewrite); rewrite only the affected passages when publishing fails or you edit. Do not read, rewrite or regenerate other issues' markdown/HTML (never open files under `html/` or `site/`). Navigation, index and other issues' pages are all updated by `ai-digest publish`.
- Do not `cat`/print big files: `ledger.json`, `candidates.jsonl`, `logs/*.log`, HTML, raw X JSON. `triage.md`, `x-raw.md` (when X recall ran) and the paper path (`paper-cands.md`; `triage-papers.md` on fallback) **must be read in full, each once** (step 3) with paged Read — do not stop after `head`/`tail`; saving tokens applies to other days' files, logs and raw data, never to the day's candidates.
- Do not re-run `ai-digest fetch --daily` (the full fetch runs once, in step 1); for a missing arXiv batch only follow `fetch/troubleshoot-arxiv.md` with `--only arxiv,hf_daily` once (output in `fetch-extra/`).
- The publish command prints about 6 summary lines; no `--verbose`; after success (`OK` + `verify check: OK` / `decrypt check: OK`) **do not verify the URL separately and do not WebFetch your own page**. On failure it prints the last 15 log lines; act on them.
- Keep reasoning restrained: decide each section's shortlist in one pass, verify, then write; do not weigh the same item repeatedly; without new information do not rephrase and re-search or re-fetch.

## 7. Bookkeeping (not sent to the reader; all done after publishing, once each)

- `runs/P/filter-log.md`: the sections `## Pushed` and `## Seen, not pushed (for the weekly to re-check)` are **appended automatically by `ai-digest ledger add --log` from the same list below** (the note column = one reason per item, so write it only there and do not hand-write these two sections). In the same Shell call you only `cat >> runs/P/filter-log.md` a `## Source status` section (failed sources, the arXiv batch arrival as one `arxiv batches: …` line, the item counts per X recall group).
- `ledger.json`: write the issue's items with `ai-digest ledger add` **once**; do not edit item by item, do not read the whole file and write it back, do not hand-write scripts:
  ```bash
  ai-digest ledger add --day P --log runs/P/filter-log.md <<'EOF'
  P|name|id|category|url1 url2|one-line reason it was pushed
  S|name|id|category|url1|why it was seen but not pushed
  EOF
  ```
  `P` = pushed in this issue, `S` = seen but not pushed yet worth a weekly re-check (an empty `id` is generated; an existing id/URL only gets its status updated and a pushed item is never downgraded). It prints one line `ledger: +N new, M updated`.
- **Save a turn and avoid losing the bookkeeping on a mid-way failure**: chain section 6's publish, the `ai-digest ledger add --log` above and the `## Source status` append into **one Shell call** with `&&`: `ai-digest publish --issue daily/P --md digest-P.md && ai-digest ledger add --day P --log runs/P/filter-log.md <<'EOF' … EOF && cat >> runs/P/filter-log.md <<'EOF' … EOF`. When publish prints `ERROR` (non-zero exit) the other two steps do not run; fix the draft per section 6 and re-run the whole chain. If it still fails after a retry and you push the HTML instead, run the last two steps on their own before pushing.

## 8. Push (one message)

```
AI Digest · Daily · M/D (month/day of `P`)
- Title: one line (one per item; with many items group them into a few lines by section)
- …

[Open the daily](https://<SITE_DOMAIN>/<SITE_SLUG>/daily/P)
```

Send only the URL and a short summary (URL without `.html`), no attachment, no preamble, no item count and no process notes. If there is really nothing worth reading, do not publish; send a single line saying so. If publishing still fails after a retry: when `ENCRYPT=1` or on any privacy/security failure, **never** attach plaintext HTML — send only "site update failed".

The result goes to the reader (as the one message the runner delivers); send nothing more in this run.
