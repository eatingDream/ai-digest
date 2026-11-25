# Security Policy

## Supported versions

Security fixes go into the latest release and `main`.

## Reporting a vulnerability

Please **do not open a public issue**. Report it privately through GitHub:
**Security → Report a vulnerability** on this repository
(<https://github.com/eatingDream/ai-digest/security/advisories/new>).

Include what you found, how to reproduce it, and the impact. You will get an acknowledgement as soon as the maintainer can; fixes are published together with an advisory once a release is available.

## Scope

In scope: the static publisher and optional client-side encryption (`publish.py`, `site/password_template.html`, `verify_decrypt.mjs`), the renderer's HTML output (`render_html.py`), the fetch pipeline, the CI workflows, and anything that could leak secrets.

Out of scope: public demo builds (including any optional public `DIGEST_DEMO_PASSWORD`) and issues that need a compromised local machine.

## If you find a leaked secret

Tell us privately through the advisory form above so it can be rotated before details become public.
