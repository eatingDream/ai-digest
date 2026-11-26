# Security Policy

## Supported versions

Security fixes go into the latest release and `main`.

## Reporting a vulnerability

Please **do not open a public issue**. Report it privately through GitHub:
**Security → Report a vulnerability** on this repository
(<https://github.com/eatingDream/ai-digest/security/advisories/new>).

Include what you found, how to reproduce it, and the impact. You will get an acknowledgement as soon as the maintainer can; fixes are published together with an advisory once a release is available.

## Scope

In scope: the static publisher, GEO/worker output, and optional client-side encryption (`src/publish.rs`, `src/geo.rs`, `src/crypto.rs`, `src/verify.rs`, `web/src/ts/`, `web/dist/verify_decrypt.mjs`), the HTML renderer (`src/render.rs`, `src/math.rs`, `src/parser.rs`), the fetch pipeline (`src/fetch/`, `src/net.rs`), the CI workflows, and anything that could leak secrets.

Out of scope: public demo builds (including any optional public `DIGEST_DEMO_PASSWORD`) and issues that need a compromised local machine.

## If you find a leaked secret

Tell us privately through the advisory form above so it can be rotated before details become public.
