<div align="center">

# AI DIGEST

**由 agent 驱动的 AI 日报 / 周报流水线：汇集约 150 个信源，渲染成单文件 HTML，再以加密静态站点的形式发布。**

[![CI](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![Python](https://img.shields.io/badge/python-3.11%2B-3776AB?logo=python&logoColor=white)
![Cloudflare Pages](https://img.shields.io/badge/deploy-Cloudflare%20Pages-F38020?logo=cloudflare&logoColor=white)

[English](README.md) | 简体中文

[在线演示](https://ai-digest-demo.pages.dev) — 密码 `demo` · [English](https://ai-digest-demo.pages.dev/en/) · [简体中文](https://ai-digest-demo.pages.dev/zh-CN/)

<img src="docs/screenshot.zh-CN.png" alt="渲染后的示例日报（合成数据）" width="860">

<sub>截图由 <code>examples/</code> 里的合成示例渲染而成。日报与页面支持中英文双语（<code>DIGEST_LANG</code>）。</sub>

</div>

## 工作流程与特性

<p align="center"><img src="docs/architecture.zh-CN.svg" alt="ai-digest 架构图" width="100%"></p>

1. **广覆盖召回与初筛（`fetch/`）** — 并行抓取约 155 个公开只读信源（`fetch/sources.toml`，零密钥），去重并用 `fetch/ledger.py` 记录已推（★）/ 看过未推（☆）历史，只有出现实质新进展才会复推；产出 `triage.md`、`triage-papers.md` 和 `candidates.jsonl`（可选按能力启用 X 只读检索 `fetch/x-recall.zh-CN.md`）。
2. **内容优先的 LLM 精选（`prompts/<语言>/`）** — 与平台无关的日报/周报提示词（含轻量论文初筛）按 `filter-notes.zh-CN.md` 核验一手出处，写成七个固定版面（模型、论文、工具与项目、行业与产品、算力与基础设施、安全与政策、观点与长文）——只看内容分量，不看热度数字。
3. **单文件 HTML 阅读器（`render_html.py`）** — 生成自包含 HTML，带完整标题侧栏目录、纯前端关键词与标签筛选、解密后 `#锚点` 自动定位、深浅色主题及中英双语支持（`DIGEST_LANG=en|zh-CN`、`--md-en` 单页双语切换，或 `LANG_ALT` 互链伴生站）。
4. **加密静态发布（`publish.py`）** — 客户端逐页 AES-256-GCM 加密（PBKDF2-SHA256，60 万次迭代，gzip），在 `_redirects` 生成静态 `/<slug>/{daily,weekly,}/latest` 跳转，发布前隐私扫描，增量部署到 Cloudflare Pages，并用 `verify_decrypt.mjs` 逐页解密校验。

## 快速开始

环境要求：Python 3.11+（CI 使用 3.12）；Node 20 与 `npx`（仅在用 `wrangler` 部署或运行 `verify_decrypt.mjs` 时需要）。

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements-dev.txt          # requests、lxml、markdown、cryptography、pytest

# 1. 渲染示例日报（加 --md-en 可将中英文合成带切换按钮的单页双语 HTML）
python3 render_html.py examples/sample-digest.zh-CN.md /tmp/sample.html --lang zh-CN
python3 render_html.py examples/sample-digest.zh-CN.md /tmp/bilingual.html --md-en examples/sample-digest.en.md

# 2. 构建加密演示站点（不部署、零密钥，构建时自动逐页解密校验）
DIGEST_DEMO_PASSWORD='demo' python3 publish.py --demo --out dist/         # 输出 dist/en/ + dist/zh-CN/（--lang 单语言）

# 3. 跑测试
pytest -q
```

### 运行你自己的日报

```bash
# 1. 配置站点（自动生成带 22 位随机 SLUG 的 site/CONFIG；可设 LANG=zh-CN 或 en）
python3 publish.py --init                                # 或 cp site/CONFIG.example site/CONFIG
install -m 600 /dev/stdin ~/.config/ai-digest/site-password   # 粘贴 16 位以上密码，Ctrl-D 结束
export CLOUDFLARE_API_TOKEN=...                          # 权限范围：Account · Cloudflare Pages · Edit

# 2. 抓取候选（先设 DIGEST_TZ；可选设 GITHUB_TOKEN/GH_TOKEN 提升 GitHub API 限额）
python3 fetch/fetch.py --daily                           # 周报用 --weekly

# 3. 交给 agent 精选：在运行器里执行 prompts/zh-CN/daily.md（或 weekly.md；英文用 prompts/en/）→ digest-<P>.md

# 4. 发布（zh-CN 站点自动发现同目录 <同名>.en.md，或显式传 --md-en；自动生成静态 latest 跳转）
python3 publish.py --issue daily/2026-01-15 --md digest-2026-01-15.md
```

抓取选项、信源频道与辅助 CLI 见 [fetch/README.md](fetch/README.md)；加密细节、`latest` 路由及 `--rebuild`、`--rerender-all`、`--rotate-password`、`--remove` 见 [site/README.md](site/README.md)。

## 配合 AI agent 使用

[AGENTS.md](AGENTS.md) 是给编码 agent 的入口（仓库地图、流水线、约定与安全规则）。要让 agent 产出日报，在带 Shell 和文件工具的运行器里打开仓库根目录，设好 `DIGEST_LANG` 与 `DIGEST_TZ`，执行 `prompts/<语言>/{papers,daily,weekly}.md`（每份开头都有运行契约；各 CLI 均支持 `--help` 与 `--json`）。X/Twitter 只读工具（`search_posts_all`、`get_users_posts` 等）为**可选增强**：没有时自动干净跳过并基于公开信源筛选；有工具时可在 `fetch/x-recall.zh-CN.md` 和 `x-sources.zh-CN.md` 替换 `<account_1>` / `<zh_account_1>`（未替换占位符会自动跳过）。

## 配置

环境变量优先于 `site/CONFIG`。

| 变量 | `CONFIG` 键 | 说明 | 默认值 |
|---|---|---|---|
| `SITE_DOMAIN` | `DOMAIN` | 站点域名 | `digest.example.com` |
| `SITE_SLUG` | `SLUG` | 不可猜测的路径前缀（至少 8 位 URL 安全字符；发布后不要再改） | 随机生成（`--init`） |
| `CF_ACCOUNT_ID` | `ACCOUNT_ID` | Cloudflare 账号 ID | — |
| `CF_ZONE_ID` | `ZONE_ID` | 域名所在的 zone（可选） | — |
| `CF_PAGES_PROJECT` | `PROJECT` | Cloudflare Pages 项目名 | `ai-digest` |
| `CLOUDFLARE_API_TOKEN` / `_FILE` | — | 部署用 token（仅需 Pages · Edit） | — |
| `DIGEST_SITE_PASSWORD_FILE` | — | 存放站点密码的文件（权限 600） | `~/.config/ai-digest/site-password` |
| `DIGEST_SITE_SALT_FILE` | — | PBKDF2 盐文件（自动生成） | `~/.config/ai-digest/site-salt` |
| `DIGEST_LANG` | `LANG` | 日报与站点界面语言：`en` 或 `zh-CN`（命令行 `--lang` 优先） | `en` |
| `DIGEST_LANG_ALT` | `LANG_ALT` | 可选的另一语言伴生站点路径/URL（如 `en:/en/`） | — |
| `DIGEST_ROOT_LATEST` | `ROOT_LATEST` | 可选开启根路径 `/latest`、`/daily/latest`、`/weekly/latest` 静态 302 跳转（`1`/`true`/`yes`/`on`；`/<slug>/...latest` 始终自动生成） | `0`（关闭） |
| `DIGEST_TZ` | — | 用来划分「自然日」的 IANA 时区 | `UTC` |
| `DIGEST_ROOT` | — | `runs/`、`html/`、`site/`、`ledger.json`、`logs/`、`state/` 工作目录 | 仓库根目录 |
| `LEDGER_PATH` | — | `ledger.json` 路径（首次 `ledger.py add` 自动创建） | `<DIGEST_ROOT>/ledger.json` |
| `DIGEST_PREFERENCES_FILE` | — | 主题偏好权重 JSON（参考 `fetch/preferences.example.json`） | `fetch/preferences.json` |
| `DIGEST_PRIVACY_PATTERNS_FILE` | — | 发布前额外检查的正则 | `site/privacy-patterns.txt` |
| `DIGEST_DEMO_PASSWORD` | — | `publish.py --demo` 使用的密码 | — |
| `GITHUB_TOKEN` / `GH_TOKEN` | — | 可选，抓取时提高 GitHub API 限额 | — |

信源清单在 `fetch/sources.toml`，筛选规则在 `filter-notes.zh-CN.md`。

## 参与贡献与许可

欢迎提 PR（`main` 受保护，详见 [CONTRIBUTING.zh-CN.md](CONTRIBUTING.zh-CN.md) 与 [SECURITY.md](SECURITY.md)）。`render_html.py` 内嵌 CSS 的设计令牌改编自 [AIHOT](https://github.com/KKKKhazix/AIHOT)（MIT，© 2026 数字生命卡兹克；见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) 与 [licenses/AIHOT-MIT.txt](licenses/AIHOT-MIT.txt)）。项目采用 [MIT](LICENSE) 许可。
