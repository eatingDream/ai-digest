# 参与贡献

感谢你愿意帮忙。[English](CONTRIBUTING.md)

## 流程与规则

- **一切改动走 PR**：`main` 对所有人受保护（含维护者 `enforce_admins`，仅允许 squash 合并以保持线性历史）。请从最新 `main` 切短期分支（`feat/`、`fix/`、`docs/`、`refactor/`、`test/`、`chore/`、`ci/`），一个 PR 只做一件事，绝不直接推送 `main` 或对公开历史 force-push。
- **Conventional Commits 与自动发版**：PR 标题遵循 [Conventional Commits](https://www.conventionalcommits.org/)（如 `fix(fetch): ...`、`feat(render): ...`、`docs: ...`）。[Release Please](https://github.com/googleapis/release-please) 会自动根据合入标题同步递增 `Cargo.toml`、`Cargo.lock`、`web/package.json`、`web/package-lock.json` 与 [CHANGELOG.md](CHANGELOG.md) 并触发 `release.yml` 发版，日常 PR 无需手改版本号或更新日志。
- **公开仓库红线**：绝不提交密钥、令牌、密码、盐值、`site/CONFIG`、真实网址/邮箱、本机路径或个人账号；示例和测试仅使用合成数据（`example.com`、虚构人名）。
- **必需检查**（分支须与 `main` 同步）：`Frontend (TS + CSS + Vitest)`、`Rust (fmt + clippy + test)`、`E2E parity & release binary hygiene`、`Secret scan (gitleaks) + sanitization guard`。合并后自动删除远端分支；配置 Cloudflare Secrets 后 `.github/workflows/demo.yml` 会自动部署演示站。
- **许可协议**：贡献内容默认以 [MIT](LICENSE) 许可发布。

## 本地检查

```bash
# 前端（Node 22+）
npm --prefix web ci
npm --prefix web run typecheck
npm --prefix web run build
git diff --exit-code web/dist/
npm --prefix web test

# Rust（1.88+）
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo fmt --check
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo clippy --all-targets -- -D warnings
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo test --locked

# 脱敏门禁（必须输出 "clean"）
scripts/check_sanitized.sh
```

无需密钥预览演示站：`cargo run -- publish --demo --out dist/`（如需预览加密模式可加 `DIGEST_DEMO_PASSWORD=demo`）。

## PR 要求

- 行为变动须补充快速离线的 Rust（`tests/*.rs`）或 Vitest（`web/tests/*.test.ts`）测试；保持依赖精简并优先使用纯 Rust crate。
- 修改 `web/src/**` 后须重新构建并提交 `web/dist/`（CI 会从空目录重建校验一致性）。
- 面向用户的文案统一写入 `src/i18n.rs`，`en` 与 `zh-CN` 两份同步补全。
- 文档默认英文；修改面向用户的文档时，同步用地道中文更新对应的 `.zh-CN.md`。

## 反馈、安全与 AI 代理

请使用 [Issue 模板](https://github.com/eatingDream/ai-digest/issues/new/choose)；安全漏洞请按 [SECURITY.md](SECURITY.md) 私下报告。AI 编码代理请先阅读 [AGENTS.md](AGENTS.md)。
