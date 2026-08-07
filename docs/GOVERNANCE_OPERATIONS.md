# Governance operations

This document is for maintainers operating Agora's governance compiler. Player-facing explanations belong in the in-app governance guide.

## Safety boundary

The compiler can run governance analysis in three modes:

| Mode | Reads GitHub | Writes monitor state | Sends monitor alerts |
| --- | --- | --- | --- |
| `off` | No | No | No |
| `read-only` | Yes | No | No |
| `monitor` | Yes | Yes | According to configured alerting |

Before running anything against production:

- confirm the repository;
- confirm the policy;
- confirm input and output state paths;
- confirm whether alerts are configured;
- confirm the working tree;
- confirm the token identity and permissions.

Never use a sandbox policy with the production state file.

The current nightly workflow sets `GOVERNANCE_MODE: monitor` and `GOVERNANCE_POLICY: production`. It validates and may commit `registry/governance/governance-state.json`, and production monitor mode requires the configured alert destination. Verify [`.github/workflows/compile.yml`](../.github/workflows/compile.yml) before every operational change rather than relying on an older runbook.

## Production state

The tracked governance state is:

```text
registry/governance/governance-state.json
```

Curator decisions are recorded separately:

```text
registry/governance/quarantine_decisions.json
```

The compiler reads curator decisions but must not silently invent or rewrite them.

State identifies the governance repository and policy. A mismatch should be treated as a configuration error, not automatically converted.

## Read-only diagnostics

Use read-only mode for investigation and dry runs. It may read current GitHub issues, reactions, and reviews, but it must not write monitor state or send monitor alerts.

Use temporary output paths and verify the exact compiler options supported by the current `compile.py --help`.

From the repository root, a production-policy diagnostic that does not write governance state is:

```powershell
$env:GITHUB_TOKEN = (gh auth token)
python compiler/compile.py `
  --skip-sign `
  --governance-mode read-only `
  --governance-policy production `
  --governance-repo agora-mc/Agora-Launcher `
  --governance-state-in registry/governance/governance-state.json `
  --out tmp/governance-read-only/registry.db
```

Do not add `--governance-state-out` for a read-only diagnostic. Use a disposable output directory and inspect `git status --short` afterward.

Before sharing diagnostic output, remove tokens, webhook URLs, private usernames, and unrelated issue content.

## Registry-vote label requirement

The compiler requires every registry vote issue to carry the `registry-vote`
label before it will parse reactions on it as votes. This applies to both the
active governance pipeline (`run_governance_pipeline`, driven by
`registry/governance/vote_issues.json`) and the legacy Pass 1 vote harvest.

Consequences of an unlabeled vote issue:

- `vote_issues.json` maps the item to an issue number, but because that issue
  lacks the `registry-vote` label, no reactions are counted, no anomaly is
  detected, and the item is never flagged `under_review` from that issue.
- The item is emitted with `status_reason = vote_issue_unlabeled` in
  `governance_summary` so the misconfiguration is visible, and the compiler
  logs a warning naming the item and issue number.

When creating or moving a canonical vote issue, add the `registry-vote` label
(normalize capitalization/lowercase is fine; exact label name must match).
Do not reuse `community-review` for vote issues — that label marks review
issues and is ignored as a vote source.

## Enabling monitor mode

Monitor mode should be activated only after:

- sandbox scenarios have passed;
- state validation passes;
- repository and policy are pinned;
- alert destinations are verified;
- duplicate-event behavior is tested;
- curator decision transitions are tested;
- the workflow's commit scope is restricted to intended state files;
- rollback and malformed-state recovery have been rehearsed;
- a maintainer has explicitly approved activation.

For monitor testing, use the sandbox repository, sandbox policy, a sandbox-owned temporary state file, and a test alert destination. Never run a local monitor command against `registry/governance/governance-state.json`; production monitor writes belong to the guarded nightly workflow.

## Curator decisions

A decision maps a stable event ID to an accepted or rejected outcome.

Operational rules:

1. inspect the event and affected reactions;
2. preserve the evidence and rationale;
3. edit the decision file through review;
4. validate JSON and event identity;
5. run read-only analysis;
6. merge the decision;
7. allow monitor mode to persist the resulting state transition.

Do not edit historical event IDs to make a decision apply.

## Malformed or stale state

When state cannot be parsed or validated:

1. stop monitor writes;
2. preserve the malformed file;
3. inspect recent Git history;
4. identify the last valid state;
5. determine whether unreviewed events would be lost;
6. restore only after curator review;
7. run the state validator;
8. run governance read-only;
9. resume monitor mode only after outputs are understood.

Do not truncate state or generate a fresh empty file merely to make CI green.

## Workflow ownership

Keep automated commit scopes narrow:

- compiled database and web exports belong in release assets;
- governance monitor commits should stage only intended governance state;
- curator decisions are maintained through review;
- loader-manifest refreshes belong to their dedicated workflow;
- no workflow should commit unrelated local build output.

## Incident record

For a governance incident, record:

```text
Detected at:
Repository and policy:
Workflow run:
Mode:
State commit before:
State commit after:
Affected event IDs:
Affected reactions:
Alert outcome:
Curator decision:
Recovery actions:
Follow-up tests:
```

Do not include secrets or private webhook information.
