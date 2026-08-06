---
description: "Run a focused local code review over the current uncommitted changes. Check security, business logic against MASTER_SPEC.md, and deploy safety."
name: "review"
agent: "agent"
---
Run a focused local code review over the current uncommitted changes. Check for:
- **Security**: least-privilege capabilities, secret handling, SQL injection, XSS via raw HTML, unsafe shell execution, unsanitized file paths.
- **Business logic**: correctness against `.kilo/plans/MASTER_SPEC.md`, schema consistency, error handling, resource cleanup.
- **Deploy safety**: new dependencies, lockfile changes, build/script paths, environment assumptions, hardcoded URLs or secrets.

Report concrete findings with file paths and line numbers. Be concise — flag only real issues and suggest minimal fixes.
