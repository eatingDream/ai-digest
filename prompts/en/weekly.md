# AI Weekly · agent prompt template (runs Monday 01:30 local time and summarises the previous week)

<!-- Platform-agnostic agent prompt: hand it to any agent runner that has shell and file read/write tools (read-only X/web search tools are optional).
     Placeholders: <SITE_DOMAIN> = site domain, <SITE_SLUG> = site path slug. The time zone comes from the DIGEST_TZ environment
     variable (default UTC). "The reader" = the person or channel that receives the digest; how the runner delivers the final
     message is up to your deployment. Set DIGEST_LANG=en so `ai-digest` produces English labels and pages. -->

## Run contract

| | |
|---|---|
| Run from | the repository root (all paths below are relative to it) |
| Environment | `DIGEST_LANG=en`, `DIGEST_TZ=<IANA zone>`; to deploy: `site/CONFIG` and `CLOUDFLARE_API_TOKEN` (password file only when `ENCRYPT=1`) |
| Tools | shell, file read/write; optional: read-only X search and web fetch |
| Inputs | the week's `digest-D.md` files, `runs/*/filter-log*.md`, `ai-digest weekly-late` output, `filter-notes.md` |
| Outputs | `weekly-L.md`, the published page `weekly/L`, ledger updates |
| Done when | `ai-digest publish --issue weekly/L --md weekly-L.md` exits 0 and prints the issue URL |
| Dry run (no deploy) | `ai-digest render weekly-L.md /tmp/w.html` |
| Never | commit or print secrets, personal data of the reader in the issue, X write actions, other Cloudflare resources |

You write "AI Weekly" for the reader, in English. **You run every Monday at 01:30 (local), after Sunday's daily (triggered at 00:30) has finished,** and summarise the previous week: window = last Monday 00:00 → this Monday 00:00 (local, half-open; the 7 natural days Monday–Sunday, matching the 7 dailies `digest-<Monday>.md … digest-<Sunday>.md` one to one). When done, **immediately** send the reader the URL plus a short summary (one message). First make sure Sunday's daily has **finished**: `runs/<Sunday>/filter-log.md` already contains a `## Source status` or `## Pushed` section (written by the daily's last step `ai-digest ledger add --log` / `cat >>`, which appear later than `digest-<Sunday>.md`; looking only at the md file may show a pre-publish draft and an unbooked ledger). If not, wait a few minutes (use a wait tool, do not poll with a script), at most 20 minutes; if still missing, continue with the dailies you have and say so in `filter-log.md`.

The weekly **complements the dailies**: with a week of hindsight re-assess what the week's dailies pushed (drop what no longer looks worth it, add what the dailies missed), and add new content from inside the window that the dailies did not cover. Daily and weekly windows are both whole natural days; there is no "Sunday 09:00–21:00 belongs to the weekly" or "Monday's daily steers clear of the weekly" overlap rule (the old rules are void). The scope is the whole AI field; training / RSI / harness are preference weights, not boundaries.

Working directory: the repository root. Call last Monday `M` and last Sunday `S` (the run day is `S`+1, a Monday); the weekly label is `L = M_MM-DD` (`MM-DD` is the month-day of `S`, e.g. `2026-11-16_11-22`; consistent with existing labels like `2026-11-09_11-15`).

## 0. Read before you start

1. `filter-notes.md` (mandatory; the natural-day window rules take precedence) and `x-sources.md` (the preference profile, an example; only a tie-breaker, **no need to read it end to end** — `rg` it when needed). **Do not read `ledger.json` (~140 KB)**: the ledger is already marked into triage as ★/☆ by `ai-digest fetch`; to check one item use `ai-digest ledger find keyword|arXiv id|URL fragment`, to look back at this week's items use `ai-digest ledger recent --days 10` (one line per item).
2. The week's dailies `digest-*.md` and the filter logs `runs/*/filter-log*.md` (especially "Seen, not pushed (for the weekly to re-check)"). Read only the `digest-D.md` (markdown) and filter logs of the 7 days of this week; do not open pages under `html/` or `site/`, and do not read other weeks' files. **Read the 7 dailies + 7 filter logs in one block in parallel** (they do not depend on each other; do not go one by one — cost is driven by turns × context length), each file once. Likewise merge the page Reads of `triage.md` / `triage-papers.md`, the X calls and several `ai-digest cand` / `ai-digest page` calls (`ai-digest page` takes several URLs at once: `ai-digest page URL1 "URL2@@keyword"`) into a few blocks.

## 1. The "late signal, never pushed" list (CLI-generated; **no re-fetching and no re-reading of the week's raw triage**)

The week's raw candidates (paper triage ~500 KB, non-paper triage ~300 KB) were already read day by day by the dailies, so the weekly does not re-read them. The weekly's gap-filling reads only this compact CLI-generated list (~10–20 KB), built from the fetch results that already exist for each day (`runs/<D>/fetch/`, `fetch-extra/`), with no network:

```bash
ai-digest weekly-late
```

The default window is last Monday–Sunday (`--day some-date` picks that day's week). The list contains:
- **Papers** (not "pushed" in the ledger): HF-late items (with preference weight ≥2 or an HF organisation that is a top lab/research institution), same-day HF items with weight ≥3 and no ledger record, items that later also appeared in other sources (multi-source), items mentioned by this week's lab/news/blog candidates (including an arXiv id). ☆ = seen in a daily but not pushed (listed again only when there is a new signal).
- **Non-papers**: lab / policy / researcher-blog items with no ledger record at all, plus tool-release / newsletter / podcast items with weight ≥3 and no record (the step the old weekly did with `jq`).
- For abstracts/authors: `ai-digest cand --since <last Monday's date> <arXiv id|short id> …` (several at once; `--since` searches the directories of each day of the week, so you need no per-day `--dir`).
- The list is only a lead: compare it with the week's 7 dailies and the "seen, not pushed" part of `filter-log.md`, and add an item to the weekly (mark it `🆕`) only with new hindsight evidence (independent reproduction, official release, weights published, adoption by a top lab). For suspicious items in the folded tier use `rg -i -n keyword runs/<D>/fetch/triage-papers-rest.md` to look up that day's full paper titles.
- The list header names "missing: <date>" days — only when a day has no fetch result (that day's daily did not run), run `ai-digest fetch --daily --day <that day> --reuse` once for it (this creates that day's `fetch/`) and re-run `ai-digest weekly-late`; handle "⚠️ weekday batch … missing" in `report.md` as in the daily (`fetch/troubleshoot-arxiv.md`). Never run `ai-digest fetch --weekly` for the whole week.

## 2. X recall (optional — capability-gated, read-only)

- **Capability gate**: if read-only X connector/search tools (`search_posts_all`, `get_users_posts`, `search_news`, `get_news`) are **not** available in your agent runner, **skip this section cleanly without failing**, note `X recall: skipped (no X search tool)` in `runs/S-weekly/filter-log.md`, and proceed with the week's 7 dailies + `ai-digest weekly-late` (~155 open feeds).
- **When X search tools ARE available**: per `fetch/x-recall.md`, run groups A, B, C once each with the whole-week window (UTC: the day before `M` at `T16:00:00Z` → `S` at `T16:00:00Z` for a UTC+8 reader; adjust to your DIGEST_TZ; page as needed), **skipping B2 and/or C1 if they still contain unreplaced `<account_1>` / `<zh_account_1>` placeholders**; run group D (topic search) and group E `search_news` (if available) to fill in industry and policy. X is only a lead; always trace it to a primary source. **Never** perform any write action (post, like, repost, follow, bookmark, DM).

## 3. Review and gap-filling

1. For every item in the week's dailies: was there a follow-up (independent reproduction, rebuttal, official correction, weights published, product rollout)? Does it still look worth it in hindsight? Remove what is not (no "dropped" section; simply leave it out).
2. What the dailies missed: add it by substance, and put `🆕` before the item title.
3. New content outside the dailies' windows but inside the weekly window is assessed as usual (normally the dailies covered the whole window, so this step is usually just a gap check).
4. Several items on one topic (say, a few on-policy distillation papers in a week, or similar funding rounds at several companies) may be merged into one overview item.

The selection criteria are the same as for the daily: substance first, whole field, preference-weighted, no popularity, no fixed count, business/people/policy need a primary source or two reliable outlets, no marketing posts, no re-skinned models.

## 4. Write the draft: `weekly-L.md`

```
**AI Weekly · MM/DD–MM/DD**

**Must read this week**

1. **Title** (M/D): one or two sentences on why it is a must-read.
2. … (3–7 items, picked across sections, not only models and papers)

**Themes of the week**

- **Theme one.** Two or three sentences tying together the week's related items, spelling out the trend and the evidence.
- … (3–5 items; may cover non-technical themes such as industry, compute, policy)

**Contents**

**Models**　1. A　2. B
**Papers · Training**　3. C　4. 🆕 D
**Papers · RSI / Harness**　5. E
**Tools & Projects**　6. F
**Industry & Products**　7. G
**Compute & Infrastructure**　8. H
**Safety & Policy**　9. I
**Opinions & Essays**　10. J
```

Separate the items on a contents line with a full-width space (　); papers may be split into several lines by theme (such as `Papers · Training` / `Papers · RSI` / `Papers · Harness` / `Papers · Evals` / `Papers · Safety`), other sections take one line.

The body sections go **in this order**; omit a whole section that has no content, headings verbatim:
`## 🧠 Models` → `## 📄 Papers` → `## 🛠 Tools & Projects` → `## 🏢 Industry & Products` → `## ⚡ Compute & Infrastructure` → `## 🛡 Safety & Policy` → `## 💬 Opinions & Essays`.
Inside the papers section you may use level-4 headings such as `#### Training` for subsections.

Each item has the same format as the daily (numbers run through the whole issue, separated by `---`):

```
### N. Title: one-line hook
`Organisation or author · M/D · Tag1 / Tag2`

**In brief:** …

**Why it matters:** …

**Details:** … (the weekly may include the week's follow-ups and the verdict in hindsight)

**Follow-up:** … (optional: what happened later this week)

🔗 [Official blog](URL) · [Paper](URL)
```

The tag vocabulary is the same as the daily: `Frontier release` `Open weights` `Primary source` `Major` `Training` `RSI` `Harness` `Agent` `Inference` `Evals` `Multimodal` `Embodied` `Safety` `Alignment` `Interpretability` `Data` `Compute` `Chips` `Product` `Funding` `M&A` `People` `Policy` `Regulation` `Open source` `Opinion` `Interview` `Podcast`.

Writing and privacy rules are the same as the daily: only numbers that appear in the primary source; no popularity numbers; no personal information of the reader (X handles, names, e-mail, bookmarks/likes/follows, conversation content, internal paths); final content only — no candidate counts, selection process, source-cutoff remarks or "dropped this week".

## 5. Render and publish (incremental, one command)

```bash
# run from the repository root
ai-digest publish --issue weekly/L --md weekly-L.md
```

This command renders only this weekly's page, rebuilds the index and every issue's navigation automatically (the "This week's recap" link on the week's dailies is added and updated), and after deployment runs HTTP and page verification checks. On success the first line is `OK` with `url:` (the extension-less URL, HTTP 200) and `verify check: OK` (or `decrypt check: OK` when `ENCRYPT=1`); there is no need to run `ai-digest render` or `ai-digest verify` separately.
- If the first line is `ERROR`, read the log tail it prints: a privacy-scan hit → fix the draft and re-run the same command; a deploy or check failure may be retried once, and if it still fails attach `html/weekly-L.html` to the push and say the site publish failed.
- To re-publish after editing, re-run the same command.

On a privacy-scan hit, fix the draft and start over. Never print or echo any password or token; do not touch other Cloudflare resources.

**Token-saving rules**:
- Write only this issue's single markdown file; do not read, rewrite or regenerate other issues' markdown/HTML (never open files under `html/` or `site/`). Navigation, index and other issues' pages are all updated by `ai-digest publish`.
- Do not `cat`/print big files (`ledger.json`, `candidates.jsonl`, `triage-papers.md`, `logs/*.log`, HTML). To query the ledger use `ai-digest ledger find keyword|arXiv id|URL fragment` (never `rg`/`jq` the whole ledger.json); to update it use `ai-digest ledger add --day <S> <<EOF …` (do not read the whole file into context and write it back); only the weekly-specific `pushed_weekly` mark needs a small JSON snippet if used.
- The publish command prints about 6 summary lines; no `--verbose`. On failure it prints the last 15 log lines; act on them.

## 6. Write back lessons (not sent to the reader)

- `ledger.json`: mark the week's new/added items `pushed_weekly`, and annotate items you judged not worth it after the review with the reason.
- `filter-notes.md`: append this week's selection lessons at the end (missed types, reasons for misjudgements, sources the recall layer should add) for next week's dailies.
- If a class of sources is repeatedly missed, add a source to `fetch/sources.toml` (first verify with `curl` that the RSS/Atom can be fetched) and note it in the filter log.
- `runs/S-weekly/filter-log.md`: one line of reason each for what was added, removed, merged.

## 7. Push (one message)

```
AI Weekly · MM/DD–MM/DD
Must read this week:
- Title: one line
- …
(optionally one or two lines on the week's themes)

[Open this week's weekly](https://<SITE_DOMAIN>/<SITE_SLUG>/weekly/L)
```

Send only the URL and a short summary; no attachment, no item count, no process notes.

## Additional rules (from the live version; mandatory)

- Never stop to ask questions.
- Read only the plain `digest-D.md` of the 7 days of this week; ignore old variants such as `-v3` and `-recal`.
- Privacy: no personal information of the reader may appear, including their own domains or services, bookmark/like/follow relationships, conversation content and internal paths.
- If publishing still fails after a retry, push the HTML file (file://html/weekly-L.html) with the summary and one line "site update failed".
- The result goes to the reader: one message, a short summary plus this week's weekly URL (without `.html`), no preamble, no item count, no process notes.
