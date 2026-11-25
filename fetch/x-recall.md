# X recall list (optional, capability-gated, read-only)

**Capability & placeholder gate**
- **Optional by runner capability**: if read-only X search tools (`search_posts_all`, `get_users_posts`, `search_news`, `get_news`) are **not** available in the agent environment (for example, standard Claude Code, Cursor, or Codex without an X connector/MCP), **skip the connector protocol cleanly without failing** — either curate directly from the ~155 open feeds in `fetch/fetch.py` or use one of the safe Grok-Bot-free alternatives below.
- **Skip unreplaced watch-account placeholders**: queries **B2** and **C1** below contain `<account_1>` / `<zh_account_1>` templates for the reader's personal circle. Before calling X search, check whether those placeholders have been replaced with real handles — if `<account_1>` or `<zh_account_1>` is still present, **skip that query** (record `B2=skipped(placeholder)` / `C1=skipped(placeholder)`) and never send literal `<...>` strings to the X API.

## Without a Grok Bot: Safe Ways to Recall X Sources (Free & Paid)

You do **not** need a Grok Bot (or any personal X login/cookies) to run AI Digest or capture X-originated AI news safely. Choose the path that fits your budget:

| Option | Cost | Account / Ban Risk | How to wire into AI Digest |
|---|---|---|---|
| **1. Open-feeds-only mode (built-in default)** | **Free** | **None** | Do nothing — `fetch/fetch.py` already pulls ~155 RSS/Atom/arXiv/HF/GitHub/lab-blog/HN/Reddit/YouTube sources and captures almost all primary launches. |
| **2. Read-only web search grounding** (`site:x.com/<handle>`) | **Free tier** (Gemini Search Grounding, Brave Search, Tavily, Exa, Perplexity) | **None** | Have your agent run 3–5 read-only web queries covering the lab handles in Section A + your date window, save verified leads to `runs/<P>/x-leads.md`, and always follow through to the primary blog/paper/repo URL. |
| **3. Open cross-posted feeds & self-hosted RSS bridges** | **Free** | **None** | Add researchers' official Bluesky feeds (`https://bsky.app/profile/<handle>/rss`), Mastodon, Substack, or self-hosted [RSSHub](https://docs.rsshub.app/) public read-only routes directly to `fetch/sources.toml` with conservative polling intervals. |
| **4. xAI Grok API (`x_search` live X tool)** *(recommended paid)* | **Paid** (pay-as-you-go API, typically pennies/day) | **None** (official API, zero X cookies) | Call xAI's Responses API (`https://api.x.ai/v1/responses`) with the built-in `x_search` tool (`allowed_x_handles`, `from_date`, `to_date`) using groups A–C below, then write `URL + one-sentence lead` lines to `runs/<P>/x-leads.md`. Store `XAI_API_KEY` in env or a `chmod 600` file outside the repo. |
| **5. Official X API v2 (Pay-Per-Use / Basic)** | **Paid** | **None** (official Bearer Token) | Call `GET /2/tweets/search/recent` with the exact queries in Sections A–D (`start_time`, `end_time`, `tweet.fields=created_at`, `expansions=author_id`, `user.fields=username`), save JSON to disk, and run `python3 fetch/xparse.py ...` verbatim. Store `X_BEARER_TOKEN` in env / `chmod 600` file. *(Note: X's official **Free** API tier does **not** include timeline or search reads.)* |
| **6. Third-party read-only X / search APIs & MCPs** | **Paid** (usage-based) | **Low** (no personal X account needed) | Use a read-only search/X provider (e.g., Exa, Tavily, Perplexity Sonar, TwitterAPI.io, or Apify) via API or MCP, normalize leads into `runs/<P>/x-leads.md`, and still trace every item to a primary source. |

**Safety red lines (never cross):**
- **Never feed personal main-account cookies (`auth_token` / `ct0`)** into headless browsers, Playwright scripts, or unofficial GraphQL scrapers — this risks permanent X account suspension and session hijacking.
- **Strictly read-only**: never automate posting, likes, bookmarks, follows, or DMs.
- **Keep keys & private circles out of the repo**: store API keys in environment variables or `chmod 600` files outside the repo; never commit credentials to `site/CONFIG` or leak private watch-circle lists into published digests.

**Call protocol (token-saving, when X search tools are available)**
1. Run every `search_posts_all` / `search_news` through the agent's read-only X search tool and have the result **written to disk (`write_to_file`, or saved to a file via shell if the connector lacks `write_to_file`), never left in the context** (previously every call dumped 6–20 KB of JSON into the context; 7–9 calls came to 100 KB+). Parameters: `max_results` 50 (groups A/B/C; the old default of 25 was truncated on A2, B1 and C1, leaving recall gaps), `start_time`/`end_time` = the window, `post.fields=["created_at"]`, `expansions=["author_id"]`, `user.fields=["username"]`; for groups B and C add `-is:reply` at the end of the query.
2. When all calls are done run the parser **exactly once**: `python3 fetch/xparse.py A1=<file> A2=<file> A3=<file> B1=… C2=… [B2=… C1=…] E1=… --out runs/D/x-raw.md` (include `B2` / `C1` only if configured). It de-duplicates across groups, strips engagement numbers, prints a first line of counts (`A1=20 A2=25 …`, copy it verbatim to the top of `x-leads.md`) and one lead per line (~200 bytes each). That single output is the set of X leads; read it once, **do not cat the raw JSON again and do not write your own parser**.
2b. **Paging after truncation**: the connector returns at most 25 posts per call, so `max_results` 50 does not always take effect. When the second line of xparse shows "⚠️ truncated": note that a full-archive search may carry a `next_token` even when it returned only a few posts (e.g. one night A1 returned only 2 posts, the earliest at 12:13, so the first 3 hours of the window were missing). For groups A and B and the account query of group C (C1, if configured), page with the `next_token` it printed (same parameters plus `next_token`; in one parallel block, again written to disk), then add the page files to the **same** xparse command and re-run it (labels like `B1p2=…`): for group A (official accounts, the most important) page until there is no `next_token`, at most 2 more pages; B and C1 only 1 more page. The keyword query of group C (C2) and the topic searches of group D are too broad (25 posts cover only a few minutes), so do not page them; pass `sort_order: "relevancy"` from the start instead.
3. Budget: A×3 + B1 + C2 = 5 core calls (+ B2 and C1 when `<account_1>` / `<zh_account_1>` are customized, up to 7 calls); run 2–3 keywords of E (`search_news`, if available) depending on how thin the "Industry & Products" and "Policy" sections are; group D is off by default and only runs 1–2 relevant topics when a section (e.g. embodied, funding, people) is clearly empty in triage + X leads, or a lead needs cross-checking. If a search brings nothing new, do not retry with a different phrasing.
4. Chase only leads worth writing: cross-check the X leads against `triage.md` (the same event is usually in both); only what is unique to X needs a primary source; in `runs/D/x-leads.md` write just link + one sentence, ≤ 25 lines.

X is not scripted; the agent searches it with a read-only X search tool (connector / API). **Read-only**: no posting, no likes, no follows, no bookmarks, no DMs.
Tools: `search_posts_all` (always pass `start_time` / `end_time` = the window, UTC ISO8601), `search_news` / `get_news` (X news aggregation), `get_users_posts`.
Add `-is:retweet` to every query; the fields are listed in the "Call protocol" above (`post.fields=created_at`, `expansions=author_id`, `user.fields=username`).
`min_likes:` and the like can only be used to de-noise during discovery, never as a reason to include an item, and never appear in the text.

## A. Official and lab accounts (run when X search is available; split into 3 groups because of the 1024-character limit)

1. `(from:OpenAI OR from:OpenAIDevs OR from:AnthropicAI OR from:claudeai OR from:GoogleDeepMind OR from:GoogleAI OR from:googlegemma OR from:GeminiApp OR from:AIatMeta OR from:MistralAI OR from:xai OR from:MSFTResearch OR from:NVIDIAAI OR from:nvidia OR from:huggingface OR from:thinkymachines OR from:cohere OR from:allen_ai OR from:togethercompute) -is:retweet -is:reply`
2. `(from:deepseek_ai OR from:Alibaba_Qwen OR from:QwenDevs OR from:Kimi_Moonshot OR from:Zai_org OR from:MiniMax_AI OR from:StepFun_ai OR from:TencentHunyuan OR from:BytedanceTalk OR from:OpenBMB OR from:intern_lm OR from:Xiaomi_MiMo OR from:SakanaAILabs OR from:perplexity_ai OR from:cursor_ai OR from:cognition OR from:windsurf OR from:Replit OR from:LangChainAI OR from:vllm_project OR from:lmsysorg OR from:llama_cpp OR from:ollama) -is:retweet -is:reply`
3. `(from:METR_Evals OR from:EpochAIResearch OR from:ArtificialAnlys OR from:lmarena_ai OR from:AISecurityInst OR from:apolloaievals OR from:GoodfireAI OR from:TransluceAI OR from:PrimeIntellect OR from:NousResearch OR from:physical_int OR from:Figure_robot OR from:UnitreeRobotics OR from:SemiAnalysis_ OR from:StanfordHAI OR from:CSETGeorgetown) -is:retweet -is:reply`

## B. Researchers / engineers / commentators (B1 always when X is available; B2 optional once customized)

1. `(from:karpathy OR from:natolambert OR from:_jasonwei OR from:polynoamial OR from:soumithchintala OR from:rasbt OR from:omarsar0 OR from:dair_ai OR from:TheTuringPost OR from:_akhaliq OR from:arankomatsuzaki OR from:iScienceLuvr OR from:teortaxesTex OR from:swyx OR from:simonw OR from:emollick OR from:Miles_Brundage OR from:jackclarkSF OR from:ylecun OR from:demishassabis OR from:sama OR from:JeffDean OR from:ilyasut OR from:drfeifei) -is:retweet`
2. *(Optional — skip if `<account_1>` is still unreplaced)* `(from:<account_1> OR from:<account_2> OR … ) -is:retweet` (the reader's own circle of researchers/engineers: put the accounts you trust here, ≤ 1024 characters per group; see `x-sources.md`)

## C. Chinese-language circle (C2 keyword query when X is available; C1 account query optional once customized)

*(C1, optional — skip if `<zh_account_1>` is still unreplaced)* `(from:<zh_account_1> OR from:<zh_account_2> OR … ) -is:retweet` (the reader's own Chinese-language circle; see the example in `x-sources.md`)
and the keyword query *(C2)*: `(发布 OR 开源 OR 上线 OR 论文 OR 融资 OR 收购) (模型 OR 大模型 OR 智能体 OR Agent OR AI) lang:zh -is:retweet` (window only, a lead only, check the primary source afterwards)

## D. Topic searches (broad; run them all when needed; each group is only a lead and must be traced to a primary source)

| Type | Query |
|---|---|
| Model releases | `("open weights" OR "open-weight" OR "now available" OR "introducing" OR "we're releasing") (model OR LLM) -is:retweet lang:en` |
| Papers | `(arxiv.org OR huggingface.co/papers) (RL OR agent OR "post-training" OR distillation OR reasoning OR harness OR "self-improving") -is:retweet` |
| Harness / agent engineering | `("agent harness" OR "coding agent" OR "Claude Code" OR Codex OR "computer use" OR MCP OR "agent skills") -is:retweet lang:en` |
| Compute / chips | `(GPU OR TPU OR Blackwell OR Rubin OR HBM OR "data center" OR datacenter OR "export controls") (AI OR NVIDIA OR AMD OR TSMC OR Huawei) -is:retweet` |
| Funding / M&A | `("raises" OR "Series A" OR "Series B" OR "Series C" OR valuation OR acquires OR acquisition) (AI OR LLM OR agent) -is:retweet lang:en` |
| Policy / safety | `("AI Act" OR "executive order" OR "AI safety" OR "AI Security Institute" OR regulation OR lawsuit) AI -is:retweet lang:en` |
| People moves | `("joins" OR "joined" OR "leaving" OR "departs" OR "steps down" OR "new CEO") (OpenAI OR Anthropic OR DeepMind OR xAI OR Meta OR NVIDIA OR "Thinking Machines" OR SSI) -is:retweet lang:en` |
| Long reads / interviews | `(podcast OR interview OR essay OR "new post" OR talk OR lecture) (AI OR LLM OR AGI) has:links -is:retweet lang:en` |
| Embodied / robotics | `(humanoid OR robotics OR "robot foundation model" OR VLA OR embodied) -is:retweet lang:en` |

## E. X news aggregation

`search_news`: `AI model`, `OpenAI`, `Anthropic`, `Google Gemini`, `NVIDIA`, `AI regulation`, `AI funding`, `DeepSeek`, `Qwen`; `get_news` for the day's tech section. Use it to fill in the "Industry & Products" and "Policy" sections.

## Principles

- An X post is only a lead: models → HF / official blog; papers → arXiv; funding/people → company announcement or reliable media; policy → official documents. If no primary source can be found, mark it "seen on X only" or leave it out.
- The reader's circle (B2, group C; example account list in `x-sources.md`) is only a preference hint and a tie-breaker, not a boundary; substantive work the circle did not mention is still included.
- Write the retrieved leads into `runs/<date>/x-leads.md` (link and one sentence only, nothing beyond the author's personal info), so the weekly can re-check them.
