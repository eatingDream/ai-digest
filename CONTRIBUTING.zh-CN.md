# 参与贡献

感谢你愿意帮忙。[English](CONTRIBUTING.md)

## 基本规则

- **一切改动都走 Pull Request。** `main` 受保护：从分支或 fork 提 PR，等必需检查通过后以 squash 方式合并，请勿直接推送到 `main`。
- **必需检查**（须全部通过）：`Python (compile + pytest)`、`Node (syntax check)`、`Secret scan (gitleaks) + sanitization guard`，且分支要与 `main` 保持同步。
- **绝不提交密钥或个人信息**：令牌、密码、盐值、`site/CONFIG`、真实站点地址、真实邮箱、本机路径、任何人的账号。示例和测试只用合成数据（`example.com`、虚构人名）。否则脱敏检查和 gitleaks 会让 PR 失败。
- 项目采用 [MIT](LICENSE) 许可；提交贡献即表示你同意以相同许可发布。

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

- 一个 PR 只做一件事，并简要说明改了什么、为什么。
- 行为变化要补测试，保持 `pytest -q` 离线且很快。
- 面向用户的文字统一放进 `i18n.py`，`en` 与 `zh-CN` 两份都要有。
- 文档以英文为默认；改了面向用户的文档，请同步更新对应的 `.zh-CN.md`，用地道的中文表达，不要直译。
- 对用户可见的改动，在 [CHANGELOG.md](CHANGELOG.md) 的 `Unreleased` 下加一行。
- 尽量不引入新依赖，确需引入时同步更新 `requirements*.txt`。

## 反馈问题与想法

请使用 [Issue 模板](https://github.com/eatingDream/ai-digest/issues/new/choose)。安全问题请看 [SECURITY.md](SECURITY.md)，不要公开提 issue。

AI 编码代理请先阅读 [AGENTS.md](AGENTS.md)。
