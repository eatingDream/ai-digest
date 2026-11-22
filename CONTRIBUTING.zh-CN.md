# 参与贡献

感谢你愿意帮忙。[English](CONTRIBUTING.md)

## 基本规则

- **一切改动都走 Pull Request。** `main` 对所有人受保护（含维护者，已启用 `enforce_admins`）：从短期分支或 fork 提 PR，等必需检查全部通过并解决 review 讨论后，以 squash 方式合并。绝不直接推送 `main`，也不要对已公开历史做 force-push。
- **必需检查**（须全部通过，且分支与 `main` 保持同步）：`Python (compile + pytest)`、`Node (syntax check)`、`Secret scan (gitleaks) + sanitization guard`。
- **这是公开仓库，绝不提交密钥或个人信息**：令牌、密码、盐值、`site/CONFIG`、真实站点地址、真实邮箱、本机路径、任何人的账号。示例和测试只用合成数据（`example.com`、虚构人名）。GitHub push protection、gitleaks 和 `scripts/check_sanitized.sh` 会共同拦截泄露。
- 项目采用 [MIT](LICENSE) 许可；提交贡献即表示你同意以相同许可发布。

## 开发迭代流程

仓库采用单主干 GitHub Flow，在 `main` 上保持干净的线性提交历史：

1. **明确范围**：较大改动建议先提 Issue 或确认目标；一个 PR 只做一件事。
2. **从最新 `main` 切分支**：`git checkout main && git pull --ff-only && git checkout -b <type>/<short-slug>`，统一使用前缀：`feat/`、`fix/`、`docs/`、`refactor/`、`test/`、`chore/`、`ci/`。
3. **Commit 与 PR 标题规范**：PR 标题（及提交信息）采用 [Conventional Commits](https://www.conventionalcommits.org/)（如 `fix(fetch): ...`、`feat(render): ...`、`docs: ...`）。`main` 仅允许 squash 合并并默认采用 PR 标题，确保每个 PR 在主干上对应一条清晰记录。
4. **本地自检后提 PR**：推送前跑完下方检查，按模板说明改了什么与为什么，关联对应 Issue（`Closes #N`）；CI 全绿并完成 Review 后 squash 合并，远端分支会自动删除。

## 自动发版流程

发版由 [Release Please](https://github.com/googleapis/release-please)（`.github/workflows/release-please.yml`）全自动管理：

1. 日常 PR 使用 Conventional Commits 标题合并到 `main`（`fix:` / `perf:` / `refactor:` 触发 patch，`feat:` 触发 minor，`!:` 或 `BREAKING CHANGE:` 在 1.0 前触发 minor、1.0 后触发 major）。贡献者**无需**手动修改 `version.py` 或 `CHANGELOG.md`。
2. 每次 `main` 更新后，Release Please 会自动创建或持续更新一个发版 PR（`chore(main): release X.Y.Z`），自动递增 [version.py](version.py)、[.release-please-manifest.json](.release-please-manifest.json) 并生成 [CHANGELOG.md](CHANGELOG.md)。
3. 准备发版时，只需确认该发版 PR 的 CI 通过后 squash 合并：Release Please 会自动创建 `vX.Y.Z` tag 与 GitHub Release，并自动调用 `.github/workflows/release.yml` 跑完测试、构建并上传源码包、演示站压缩包及 `SHA256SUMS.txt`。
4. 如需发布预发布版本（如 `v0.2.0-rc.1`），仍可在 `main` 上手动推送对应 tag 直接触发 `.github/workflows/release.yml`。

## 本地环境与测试

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements-dev.txt
python -m py_compile *.py fetch/*.py tests/*.py
pytest -q                          # 离线运行，几秒即可
node --check verify_decrypt.mjs
scripts/check_sanitized.sh         # 必须输出 "clean"
```

不需要任何密钥就能预览演示站：`DIGEST_DEMO_PASSWORD=demo python3 publish.py --demo --out dist/`。

## 提 PR 前自查

- 一个 PR 只做一件事，使用 Conventional Commits 标题，并简要说明改了什么、为什么（[CHANGELOG.md](CHANGELOG.md) 会根据 PR 标题自动生成）。
- 行为变化要补测试，保持 `pytest -q` 离线且很快。
- 面向用户的文字统一放进 `i18n.py`，`en` 与 `zh-CN` 两份都要有。
- 文档以英文为默认；改了面向用户的文档，请同步更新对应的 `.zh-CN.md`，用地道的中文表达，不要直译。
- 尽量不引入新依赖，确需引入时同步更新 `requirements*.txt`。

## 反馈问题与想法

请使用 [Issue 模板](https://github.com/eatingDream/ai-digest/issues/new/choose)。安全问题请看 [SECURITY.md](SECURITY.md)，不要公开提 issue。

AI 编码代理请先阅读 [AGENTS.md](AGENTS.md)。
