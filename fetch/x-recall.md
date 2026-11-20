# X recall list (for scheduled runs, read-only)

**Call protocol (token-saving)**
1. Run every `search_posts_all` / `search_news` through the agent's read-only X search tool and have the result **written to disk (`write_to_file`), never into the context** (previously every call dumped 6–20 KB of JSON into the context; 7–9 calls came to 100 KB+). Parameters: `max_results` 50 (groups A/B/C; the old default of 25 was truncated on A2, B1 and C1, leaving recall gaps), `start_time`/`end_time` = the window, `post.fields=["created_at"]`, `expansions=["author_id"]`, `user.fields=["username"]`; for groups B and C add `-is:reply` at the end of the query.
2. When all calls are done run the parser **exactly once**: `python3 fetch/xparse.py A1=<file> A2=<file> A3=<file> B1=… B2=… C1=… C2=… E1=… --out runs/D/x-raw.md`. It de-duplicates across groups, strips engagement numbers, prints a first line of counts (`A1=20 A2=25 …`, copy it verbatim to the top of `x-leads.md`) and one lead per line (~200 bytes each). That single output is the set of X leads; read it once, **do not cat the raw JSON again and do not write your own parser**.
2b. **Paging after truncation**: the connector returns at most 25 posts per call, so `max_results` 50 does not always take effect. When the second line of xparse shows "⚠️ truncated": note that a full-archive search may carry a `next_token` even when it returned only a few posts (e.g. one night A1 returned only 2 posts, the earliest at 12:13, so the first 3 hours of the window were missing). For groups A and B and the account query of group C (C1), page with the `next_token` it printed (same parameters plus `next_token`; in one parallel block, again via `write_to_file`), then add the page files to the **same** xparse command and re-run it (labels like `B1p2=…`): for group A (official accounts, the most important) page until there is no `next_token`, at most 2 more pages; B and C1 only 1 more page. The keyword query of group C (C2) and the topic searches of group D are too broad (25 posts cover only a few minutes), so do not page them; pass `sort_order: "relevancy"` from the start instead.
3. Budget: A×3 + B×2 + C×2 = 7 mandatory calls; run 2–3 keywords of E (`search_news`) depending on how thin the "Industry & Products" and "Policy" sections are; group D is off by default and only runs 1–2 relevant topics when a section (e.g. embodied, funding, people) is clearly empty in triage + X leads, or a lead needs cross-checking. If a search brings nothing new, do not retry with a different phrasing.
4. Chase only leads worth writing: cross-check the X leads against `triage.md` (the same event is usually in both); only what is unique to X needs a primary source; in `runs/D/x-leads.md` write just link + one sentence, ≤ 25 lines.

X is not scripted; the agent searches it with a read-only X search tool (connector / API). **Read-only**: no posting, no likes, no follows, no bookmarks, no DMs.
Tools: `search_posts_all` (always pass `start_time` / `end_time` = the window, UTC ISO8601), `search_news` / `get_news` (X news aggregation), `get_users_posts`.
Add `-is:retweet` to every query; the fields are listed in the "Call protocol" above (`post.fields=created_at`, `expansions=author_id`, `user.fields=username`).
`min_likes:` and the like can only be used to de-noise during discovery, never as a reason to include an item, and never appear in the text.

## A. Official and lab accounts (mandatory every run; split into 3 groups because of the 1024-character limit)

1. `(from:OpenAI OR from:OpenAIDevs OR from:AnthropicAI OR from:claudeai OR from:GoogleDeepMind OR from:GoogleAI OR from:googlegemma OR from:GeminiApp OR from:AIatMeta OR from:MistralAI OR from:xai OR from:MSFTResearch OR from:NVIDIAAI OR from:nvidia OR from:huggingface OR from:thinkymachines OR from:cohere OR from:allen_ai OR from:togethercompute) -is:retweet -is:reply`
2. `(from:deepseek_ai OR from:Alibaba_Qwen OR from:QwenDevs OR from:Kimi_Moonshot OR from:Zai_org OR from:MiniMax_AI OR from:StepFun_ai OR from:TencentHunyuan OR from:BytedanceTalk OR from:OpenBMB OR from:intern_lm OR from:Xiaomi_MiMo OR from:SakanaAILabs OR from:perplexity_ai OR from:cursor_ai OR from:cognition OR from:windsurf OR from:Replit OR from:LangChainAI OR from:vllm_project OR from:lmsysorg OR from:llama_cpp OR from:ollama) -is:retweet -is:reply`
3. `(from:METR_Evals OR from:EpochAIResearch OR from:ArtificialAnlys OR from:lmarena_ai OR from:AISecurityInst OR from:apolloaievals OR from:GoodfireAI OR from:TransluceAI OR from:PrimeIntellect OR from:NousResearch OR from:physical_int OR from:Figure_robot OR from:UnitreeRobotics OR from:SemiAnalysis_ OR from:StanfordHAI OR from:CSETGeorgetown) -is:retweet -is:reply`

## B. Researchers / engineers / commentators (mandatory every run, 2 groups)

1. `(from:karpathy OR from:natolambert OR from:_jasonwei OR from:polynoamial OR from:soumithchintala OR from:rasbt OR from:omarsar0 OR from:dair_ai OR from:TheTuringPost OR from:_akhaliq OR from:arankomatsuzaki OR from:iScienceLuvr OR from:teortaxesTex OR from:swyx OR from:simonw OR from:emollick OR from:Miles_Brundage OR from:jackclarkSF OR from:ylecun OR from:demishassabis OR from:sama OR from:JeffDean OR from:ilyasut OR from:drfeifei) -is:retweet`
2. `(from:<account_1> OR from:<account_2> OR … ) -is:retweet` (the reader's own circle of researchers/engineers: put the accounts you trust here, ≤ 1024 characters per group; see `x-sources.md`)

## C. Chinese-language circle (mandatory every run)

`(from:<zh_account_1> OR from:<zh_account_2> OR … ) -is:retweet` (the reader's own Chinese-language circle; see the example in `x-sources.md`)
and the keyword query: `(发布 OR 开源 OR 上线 OR 论文 OR 融资 OR 收购) (模型 OR 大模型 OR 智能体 OR Agent OR AI) lang:zh -is:retweet` (window only, a lead only, check the primary source afterwards)

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
