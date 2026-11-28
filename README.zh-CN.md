<div align="center">

<img src="docs/logo.svg" alt="AI Digest 图标" width="64" height="64">

# AI Digest

**基于单一 Rust 二进制与 TypeScript 杂志级阅读器的 AI 日报 / 周报流水线：汇集约 155 个公开信源，渲染为单文件页面，发布为纵深加固、GEO 就绪的静态站点。**

[![CI](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/eatingDream/ai-digest?color=2952cc)](https://github.com/eatingDream/ai-digest/releases)
![Rust](https://img.shields.io/badge/rust-1.88%2B-3b82f6?logo=rust&logoColor=white)
![TypeScript](https://img.shields.io/badge/typescript-5.x-3178c6?logo=typescript&logoColor=white)
[![License: MIT](https://img.shields.io/badge/license-MIT-0f766e)](LICENSE)

[English](README.md) | 简体中文 · [在线演示](https://ai-digest-demo.pages.dev) · [版本发布](https://github.com/eatingDream/ai-digest/releases) · [Agent 指南（`llms.txt`）](llms.txt)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshot-dark.zh-CN.png">
  <source media="(prefers-color-scheme: light)" srcset="docs/screenshot.zh-CN.png">
  <img src="docs/screenshot.zh-CN.png" alt="AI Digest 渲染示例页面（合成数据）" width="880">
</picture>

</div>

## 它是什么（不是什么）

**AI Digest** 是一套开源自包含的 Rust 命令行工具（`ai-digest`）与现代杂志级单文件阅读器：负责从公开渠道召回前沿 AI 资讯、驱动任意 LLM Agent 精选成日报/周报 Markdown，并一键发布为快速且经过隐私扫描的 Cloudflare Pages 静态站点。
它**不是**托管 SaaS 订阅服务、不依赖个人 Cookie 爬虫，也不是笨重的前端框架——所有能力打包在单个原生二进制与零外部依赖的单文件 HTML 中。

## 核心特性

- **零依赖单文件 Rust 二进制** — 内置异步信源召回（`fetch`）、去重账本（`ledger`）、精选辅助（`cand`、`page`、`xparse`）、单文件渲染（`render`）、静态站与 GEO 发布（`publish`）及双运行时回测（`verify`），无需 Python 或 Node 运行时。
- **宽幅报刊编辑阅读器** — 配置驱动的静夜靛蓝（Lucid Indigo）深浅双主题（`web/ui-theme.json`、零白屏闪烁与圆形视图过渡动画）、导读摘要、高亮「头条」条目与多信源引用胶囊、三档字号切换（`小` / `中` / `大`）、默认全展开支持 `Cmd+F` 检索的侧栏目录、距离自适应瞬达跳转、中英切换锚点锁定防跳动、统一胶囊搜索抽屉（`/`）、键盘导航（`j`/`k`/`[`/`]`）与构建期行内 LaTeX 数学公式。
- **原生双语路由与静态 `latest`** — 自动配对 `<同名>.en.md` 生成对称 `/zh/` 与 `/en/` 站点目录（亦支持 `--md-en` 单页双语），输出 `hreflang`（含 `x-default`）多语言关联，并直接提供 HTTP 200 `/daily/latest` 与 `/weekly/latest`。
- **默认公开的 Level 5/5 GEO 与 Agent 就绪** — 自动输出 `robots.txt` Content Signals、`sitemap.xml`、`llms.txt`/`llms-full.txt`、Atom（`/feed.xml`）/ RSS 2.0（`/rss.xml`）/ JSON Feed 1.1（`/feed.json`）订阅、JSON-LD 结构化数据、全站 `.md` 镜像、`/.well-known/api-catalog`（RFC 9727）、Agent Skills 索引与 Cloudflare Pages `_worker.js`（支持 `Accept: text/markdown` 与 `GET`/`HEAD`/`QUERY` `/.well-known/search`）。
- **纵深安全与可选客户端加密** — 构建期 SHA-256 哈希锁定 CSP（零 `'unsafe-inline'` 脚本）、严格安全响应头、Markdown XSS 净化、HTML5 实体解码隐私扫描，以及一条参数即可开启的客户端 AES-256-GCM 加密（`--init --encrypt`）。
- **约 155 个零密钥公开信源** — 开箱抓取 RSS/Atom、arXiv、Hugging Face、GitHub、实验室官博、HN 与 Reddit，X 召回完全可选且无需交出个人 Cookie。

## 快速开始

从 [GitHub Releases](https://github.com/eatingDream/ai-digest/releases) 下载预编译二进制，或用 Rust 1.88+ 从源码构建：

```bash
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo build --release --locked
./target/release/ai-digest publish --demo --out dist/          # 本地构建双语演示站（零密钥）
./target/release/ai-digest render examples/sample-digest.zh-CN.md /tmp/sample.html --lang zh-CN
```

运行自己的日报 / 周报流水线：

```bash
ai-digest publish --init && ai-digest fetch --daily
# 交由 AI Agent 按 prompts/zh-CN/daily.md 精选输出 digest-YYYY-MM-DD.md
ai-digest publish --issue daily/YYYY-MM-DD --md digest-YYYY-MM-DD.md
```

## 面向 AI Agent

AI Digest 原生面向编码 Agent（Claude Code、Codex、Cursor、OpenCode、Antigravity）协作与消费：

```bash
npx skills add eatingDream/ai-digest
```

直接让 Agent 阅读 [`llms.txt`](llms.txt)、[`skills/ai-digest/SKILL.md`](skills/ai-digest/SKILL.md)、[`docs/ai-guide.md`](docs/ai-guide.md)（完整环境变量 / `CONFIG` 配置表、CLI 矩阵与安全 X 召回指南）或 [`AGENTS.md`](AGENTS.md)。每个公开部署站均内置：

- `/llms.txt`、`/llms-full.txt` 与 `/.well-known/api-catalog`（RFC 9727 端点目录）
- `Accept: text/markdown` 内容协商、单页 `.md` 镜像与 `/.well-known/search?q=<query>`
- `/feed.xml`（Atom）、`/rss.xml`（RSS 2.0）、`/feed.json`（JSON Feed 1.1）、含 `hreflang` 的 `sitemap.xml` 与 JSON-LD 结构化数据

## 安全与隐私

- **防 SSRF 与只读抓取** — DNS 绑定安全 HTTP 拦截内网/回环地址，绝不使用个人社交账号 Cookie。
- **零密钥仓库卫生** — 凭据严格存放在仓库外 `0600` 文件；发布前执行 HTML5 实体解码隐私扫描拦截敏感泄露。
- **哈希锁定严格 CSP** — 所有可执行内联脚本在构建期计算 SHA-256 写入 `_headers`，零 `'unsafe-inline'` 脚本。
- **加密模式严格隔离** — 开启 `ENCRYPT=1` 时自动关闭公开 GEO 端点并强制 `Disallow: /` 与 `noindex`。

## 文档索引

- [架构图](docs/architecture.zh-CN.svg) · [Agent 与运维配置指南](docs/ai-guide.md) · [AGENTS.md](AGENTS.md)
- [信源与抓取说明](fetch/README.md) · [站点与发布说明](site/README.md) · [无 Grok Bot 安全抓 X](fetch/x-recall.zh-CN.md)
- [参与贡献](CONTRIBUTING.zh-CN.md) · [安全策略](SECURITY.md) · [第三方声明](THIRD_PARTY_NOTICES.md)

## 许可

[MIT](LICENSE) — CSS 设计令牌改编自 [AIHOT](https://github.com/KKKKhazix/AIHOT)（MIT，详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)）。
