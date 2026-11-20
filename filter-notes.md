# Selection notes (filter-notes)

The agent reads this file before every run. The weekly appends to it at the end and the daily uses it. What follows are **generic rules that
do not depend on any person**; add or remove rules to match your own preferences. Personal content (preference profile, named accounts/items)
belongs in a local file that is not committed.

## Must follow

1. **Never use stars, votes, downloads or chart rank as a reason to include or drop an item.** Dropping substantive results only because they were "not popular enough" is an explicit mistake.
2. **Do not re-push an existing tool just because it "trended" when there was no real progress this week.**
3. **A lab blog that will not open is not evidence of "nothing released today".** Try another entry point (homepage, official X account, GitHub, HF org) and write "did not open, may have missed" in the filter log.
4. **Scores of one model on different benchmark versions must not be mixed** (e.g. benchmark 4.0 and 4.1 are two benchmarks).
5. **A paper whose abstract has no concrete numbers may stay because the problem it addresses matters, but the text must not invent deltas**; write only numbers checked in the abstract or body.
6. **The preference profile is a preference hint only, never a primary source**, and community buzz must not automatically become a must-push; conversely, when substantive work pointed out by the community was wrongly dropped, the weekly must bring it back.
7. **Do not dismiss an official release claim just because you only saw a GitHub pre-release tag**; open the official blog/announcement first, then judge whether "this really happened".
8. **Lab research pages and new agent benchmarks do not always show up on arXiv or HF Daily**; look separately in official blogs / GitHub / news for releases of the kind "new benchmark, fixed harness, only the model changes".
9. **The effective date of policy text (terms of service, supported regions, usage policy) is news too**; for changes to a vendor's legal/policy pages look at the effective date.
10. **Quantitative long reads from research organisations are judged by paper standards**, not skipped because they are "blog/opinion".
11. **When assessing self-improvement / harness papers, first check whether the grader is visible to the agent and whether results hold when a different grader re-scores them.**
12. **A lab's incident / misalignment report pages often have no RSS**; open the section page directly from time to time.

## Scope and trade-offs

13. **The scope is the whole AI field**: product launches, business (funding/M&A/revenue), people moves, policy and regulation, compute and chips, embodied robotics, multimodal, safety and alignment, long reads/interviews/podcasts — the standard is the same as for papers: substance.
14. **Still excluded**: pure marketing and customer stories, minor version bug fixes, rumours without a primary source, old projects that merely trended, re-skinned quantised/merged models, tech news unrelated to AI.
15. **Business/people/policy items must have a primary source or cross-checking by two reliable outlets** (company announcement, regulatory document, official document, the person's own post); when only a paywalled headline exists write "according to X" and add no detail.
16. **Open weights going from "announced" to "downloadable" is real progress** and may justify another item on a model that was already pushed.

## Valid positive signals (not popularity)

- Methods compared after aligning compute or budget.
- Self-improvement results that state the verification cost and the false-accept rate.
- A lab turning the harness itself into a product or benchmark.

## Schedule and windows

17. **Daily and weekly windows are both local natural days, half-open `[start,end)`** (the time zone comes from `DIGEST_TZ`, default UTC). The daily runs early each morning and produces the issue of "yesterday" `P`: window `P 00:00 → P+1 00:00`; issue number / file name / title / URL all use `P`. The weekly runs early Monday (after Sunday's daily), window = last Monday 00:00 → this Monday 00:00, covering 7 dailies, label `Monday_Sunday` (e.g. `2026-01-12_01-18`). There is no overlap rule between dailies.
18. **arXiv batches**: the list dated D opens at 20:00 US Eastern on the previous day (Sunday to Thursday evenings) and falls inside that day's window; by the early-morning run it is long online, so do not wait or backfill. **When `P` is a weekday and there is no arXiv batch, that is a real recall failure** (or an arXiv holiday): re-check once and find the cause; it is not "no papers". No batch on Saturday/Sunday is normal.
19. **HF Daily**: boards are published by UTC date and there is none at weekends; the daily takes every UTC date whose board is complete at run time and was not fetched before (usually the whole board of P-1).

## Process (cost-optimised; the selection standard itself is unchanged)

20. **The "Rest · titles only" tier of the paper triage is folded, and paper nomination moved up front into the papers-prefilter step.**
    - In `triage-papers.md` off-topic (preference weight 0) papers keep only those with a signal (HF / multi-source / mentioned by other sources / ledger / big-result keywords: technical report, frontier model name, open weights, scale, hard benchmark/math, safety/adversarial); the rest is folded into one count line. The **full titles** are in `triage-papers-rest.md` next to it; when you suspect a missed class of big results spot-check with `rg -i -n 'keyword' …/triage-papers-rest.md` (a safety net, no full read needed).
    - The prefilter step reads the whole paper triage and writes `runs/P/paper-cands.md` (a loose list of 30–40 papers, err on the wide side); the daily reads it instead of the whole paper triage, and falls back to `triage-papers.md` when it does not exist. The daily must not skip a paper it met in the non-paper leads merely because the prefilter did not list it.
    - The prefilter and the daily share one fetch (`fetch.py --daily --reuse`) and do not hit the network twice.
21. **The weekly neither re-fetches nor re-reads the week's raw triage.** It reads the 7 dailies + 7 filter logs + `ledger.py recent` + the "late signal, never pushed" list generated by `weekly_late.py`. For abstracts use `cand.py --since <Monday>`.
