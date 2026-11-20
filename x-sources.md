# X source profile (example template)

Purpose of this file: give the agent a "reader preference profile" that is **only used as a tie-breaker when two candidates of equal substance are hard to rank**.
It is not an inclusion bar and not a recall list (the recall list is in `fetch/x-recall.md`). Replace the placeholders below with your own content;
a file with personal preferences should not be committed to a public repository (rename it `x-sources.local.md` and add it to `.gitignore`).

## 1. Accounts you follow (examples; replace with your own)

### Labs and official accounts
- Official and developer accounts of frontier / open-model labs (OpenAI, Anthropic, Google DeepMind, Meta AI, Mistral, DeepSeek, Qwen, Hugging Face, …)

### Researchers and paper curators
- Researchers / engineers who regularly post paper digests, benchmarks, evals and engineering practice (`<account_1>`, `<account_2>` …)

### Local inference and open-source engines
- Accounts of open-source projects around inference engines / quantisation / local deployment

### Chinese-language circle (optional)
- Chinese-language AI news and paper-digest accounts (`<zh_account_1>` …)

## 2. Preferred topics (consistent with the weights in `fetch/preferences.json`)

1. Training and post-training (RL, distillation, data)
2. Agent / harness engineering
3. Inference and deployment
4. Evals and benchmarks
5. Frontier model releases

## 3. Noise to filter out

- Usage-environment noise (proxies / VPS / account sharing), marketing posts, tech news unrelated to AI, pure popularity reposts

## 4. How to use it

- Recall covers the whole field: **the first criterion for inclusion is the substance of the item itself** (new results, new methods, serious releases, evidence, practical value);
  being mentioned by your circle does not mean it must be included, and substantive work your circle did not mention is still included.
