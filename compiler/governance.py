#!/usr/bin/env python3
from __future__ import annotations

"""
Governance Sandbox module.

Modes
-----
off        : No GitHub reads, no state file, no Discord.
read-only  : Read GitHub issues/reactions, no state file, no Discord.
monitor    : Read GitHub, update state, Discord alerts for new/grown/resolved.

Files
-----
vote_issues.json          : INPUT. registry_root/governance/vote_issues.json
                              {schema_version:1, items:{item_id:{issue_number:N}}}
governance-state.json     : IN/OUT. Events with status.
                              {schema_version:1, events:[{event_id,item_id,
                               event_type,status,detected_at,affected_reactions,
                               details_json}]}
quarantine_decisions.json : INPUT only (curator). Compiler never writes.
                              {schema_version:1, decisions:[{event_id,status}]}
"""

import hashlib
import json
import logging
import os
import re
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from enum import Enum
from pathlib import Path
from typing import Any

logger = logging.getLogger("compiler.governance")

# ---------------------------------------------------------------------------
# Enums
# ---------------------------------------------------------------------------

class GovernancePolicy(Enum):
    PRODUCTION = "production"
    SANDBOX = "sandbox"

class GovernanceMode(Enum):
    OFF = "off"
    READ_ONLY = "read-only"
    MONITOR = "monitor"

class DecisionStatus(Enum):
    PENDING = "pending"
    REJECTED = "rejected"
    ACCEPTED = "accepted"

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

COMMUNITY_REVIEW_LABEL = "community-review"
VOTE_DIRECTIONS = frozenset({"+1", "-1"})
REVIEW_MIN_LENGTH = 100
PRODUCTION_ACCOUNT_AGE_DAYS = 30
SANDBOX_ACCOUNT_AGE_DAYS = 0
PRODUCTION_RAID_THRESHOLD = 5
PRODUCTION_RAID_WINDOW_MINUTES = 360
SANDBOX_RAID_THRESHOLD = 3
SANDBOX_RAID_WINDOW_MINUTES = 10

# ---------------------------------------------------------------------------
# Repo resolution
# ---------------------------------------------------------------------------

def resolve_governance_repo(cli_value: str | None) -> str | None:
    return (
        cli_value
        or os.environ.get("AGORA_GOVERNANCE_REPO")
        or os.environ.get("AGORA_REGISTRY_REPO")
        or os.environ.get("GITHUB_REPOSITORY")
    )

# ---------------------------------------------------------------------------
# Policy config
# ---------------------------------------------------------------------------

@dataclass
class PolicyConfig:
    account_age_days: int = PRODUCTION_ACCOUNT_AGE_DAYS
    raid_threshold: int = PRODUCTION_RAID_THRESHOLD
    raid_window_minutes: int = PRODUCTION_RAID_WINDOW_MINUTES
    use_baseline: bool = True

def build_policy_config(policy: GovernancePolicy) -> PolicyConfig:
    if policy == GovernancePolicy.SANDBOX:
        return PolicyConfig(
            account_age_days=SANDBOX_ACCOUNT_AGE_DAYS,
            raid_threshold=SANDBOX_RAID_THRESHOLD,
            raid_window_minutes=SANDBOX_RAID_WINDOW_MINUTES,
            use_baseline=False,
        )
    return PolicyConfig()

# ---------------------------------------------------------------------------
# Digest / event_id
# ---------------------------------------------------------------------------

def make_event_id(item_id: str, issue_number: int, sorted_ids: list[int]) -> str:
    raw = f"{item_id}:{issue_number}:{','.join(str(i) for i in sorted_ids)}"
    return "sha256:" + hashlib.sha256(raw.encode("utf-8")).hexdigest()

def make_stable_event_id(item_id: str, issue_number: int, anomaly_detected_at: str) -> str:
    raw = f"{item_id}:{issue_number}:anomaly:{anomaly_detected_at}"
    return "sha256:" + hashlib.sha256(raw.encode("utf-8")).hexdigest()

# ---------------------------------------------------------------------------
# Issue form parsing (exact heading labels)
# ---------------------------------------------------------------------------

_HEADING_RE = re.compile(r"^###\s+(.+)")

_KNOWN_FIELDS = {
    "mod registry id": "mod_registry_id",
    "registry item id": "registry_item_id",
    "project version tested": "item_version",
    "minecraft version": "minecraft_version",
    "loader or platform": "loader",
    "your relationship to this project": "relationship",
    "review focus": "focus",
    "supporting evidence": "evidence",
    "known limitations or conflicts": "limitations",
    "technical review": "technical_review",
}

def parse_issue_form(body: str | None) -> dict[str, str]:
    result: dict[str, str] = {}
    if not body:
        return result
    lines = body.split("\n")
    current_heading: str | None = None
    content_lines: list[str] = []
    for raw_line in lines:
        line = raw_line.rstrip("\r")
        m = _HEADING_RE.match(line)
        if m:
            if current_heading is not None and content_lines:
                _save_form_field(result, current_heading.strip(), content_lines)
            current_heading = m.group(1)
            content_lines = []
        elif current_heading is not None:
            content_lines.append(line)
    if current_heading is not None and content_lines:
        _save_form_field(result, current_heading.strip(), content_lines)
    return result

def _save_form_field(result: dict[str, str], heading: str, lines: list[str]) -> None:
    content = "\n".join(lines).strip()
    h_norm = re.sub(r"\s+", " ", heading).strip().lower()
    for pattern, key in _KNOWN_FIELDS.items():
        if pattern in h_norm:
            result[key] = content
            return
    result[h_norm] = content

# ---------------------------------------------------------------------------
# Injectable fetch helpers (tests)
# ---------------------------------------------------------------------------

INJECTED_FETCH_ISSUES = None
INJECTED_FETCH_REACTIONS = None
INJECTED_USER_PROFILES: dict[str, dict] | None = None

def _fetch_issues(owner: str, repo: str, *, token: str) -> list[dict[str, Any]]:
    if INJECTED_FETCH_ISSUES is not None:
        return INJECTED_FETCH_ISSUES(owner, repo, token=token)
    import requests
    headers = _gh_headers(token)
    out: list[dict[str, Any]] = []
    page = 1
    while page <= 10:
        resp = requests.get(
            f"https://api.github.com/repos/{owner}/{repo}/issues",
            headers=headers, params={"state": "all", "per_page": 100, "page": page}, timeout=60,
        )
        if resp.status_code not in (200,):
            raise RuntimeError(f"GitHub API /issues returned {resp.status_code}")
        batch = resp.json()
        if not isinstance(batch, list):
            break
        out.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    return out

def _fetch_reactions(owner: str, repo: str, issue_number: int, *, token: str) -> list[dict[str, Any]]:
    if INJECTED_FETCH_REACTIONS is not None:
        return INJECTED_FETCH_REACTIONS(owner, repo, issue_number, token=token)
    import requests
    headers = _gh_headers(token)
    out: list[dict[str, Any]] = []
    page = 1
    while page <= 5:
        resp = requests.get(
            f"https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}/reactions",
            headers=headers, params={"per_page": 100, "page": page}, timeout=60,
        )
        if resp.status_code not in (200,):
            raise RuntimeError(f"GitHub API /reactions returned {resp.status_code}")
        batch = resp.json()
        if not isinstance(batch, list):
            break
        out.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    return out

def _fetch_user_profile(login: str, *, token: str) -> dict[str, Any] | None:
    if INJECTED_USER_PROFILES is not None:
        return INJECTED_USER_PROFILES.get(login)
    import requests
    try:
        resp = requests.get(
            f"https://api.github.com/users/{login}",
            headers=_gh_headers(token), timeout=30,
        )
        if resp.status_code == 200:
            return resp.json()
        return None
    except Exception:
        return None

def _gh_headers(token: str) -> dict[str, str]:
    return {
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "AgoraCompiler/1.0",
    }

# ---------------------------------------------------------------------------
# Label helpers
# ---------------------------------------------------------------------------

def _get_label_names(labels: list[Any]) -> list[str]:
    out: list[str] = []
    for l in labels:
        if isinstance(l, dict):
            name = l.get("name", "")
            if name:
                out.append(name)
        elif isinstance(l, str):
            out.append(l)
    return out

# ---------------------------------------------------------------------------
# Review extraction (exact enriched fields)
# ---------------------------------------------------------------------------

def extract_reviews(
    issues: list[dict[str, Any]],
    known_mod_ids: set[str],
) -> dict[str, list[dict[str, Any]]]:
    result: dict[str, list[dict[str, Any]]] = {mid: [] for mid in known_mod_ids}
    for issue in issues:
        issue_number = issue.get("number")
        if not isinstance(issue_number, int):
            continue
        labels = issue.get("labels", [])
        label_names = _get_label_names(labels)
        if COMMUNITY_REVIEW_LABEL not in label_names:
            continue
        author_obj = issue.get("user") or {}
        author = (author_obj.get("login") or "").strip().lower()
        if not author:
            continue
        form = parse_issue_form(issue.get("body"))
        mod_id = form.get("mod_registry_id", "").strip().lower()
        registry_item_id = form.get("registry_item_id", "").strip().lower()
        target_id = registry_item_id or mod_id
        if not target_id or target_id not in known_mod_ids:
            continue
        tech = form.get("technical_review", "").strip()
        if len(tech) < REVIEW_MIN_LENGTH:
            continue
        required_fields = ("item_version", "minecraft_version", "loader", "relationship", "focus")
        if any(not form.get(field_name, "").strip() for field_name in required_fields):
            continue
        created_at = issue.get("created_at", "")
        enriched = {
            "issue_number": issue_number,
            "author": author,
            "mod_id": target_id,
            "technical_review": tech,
            "item_version": form.get("item_version", ""),
            "minecraft_version": form.get("minecraft_version", ""),
            "loader": form.get("loader", ""),
            "relationship": form.get("relationship", ""),
            "focus": [value.strip() for value in form.get("focus", "").split(",") if value.strip()],
            "evidence": form.get("evidence", ""),
            "limitations": form.get("limitations", ""),
            "issue_url": f"https://github.com/issues/{issue_number}",
            "created_at": created_at,
            "labels": label_names,
        }
        result.setdefault(target_id, []).append(enriched)
    for item_id in list(result.keys()):
        entries = result[item_id]
        seen: dict[str, dict[str, Any]] = {}
        for r in entries:
            existing = seen.get(r["author"])
            if existing is None or r["created_at"] > existing["created_at"]:
                seen[r["author"]] = r
        result[item_id] = list(seen.values())
    return result

# ---------------------------------------------------------------------------
# Vote tally (raw counts, conflict detection)
# ---------------------------------------------------------------------------

def tally_item_votes(
    reactions: list[dict[str, Any]],
    blacklist: set[str],
) -> dict[str, Any]:
    user_up_ids: dict[str, list[int]] = {}
    user_down_ids: dict[str, list[int]] = {}
    all_reaction_ids: list[int] = []
    for r in reactions:
        content = r.get("content", "")
        if content not in VOTE_DIRECTIONS:
            continue
        reaction_id = r.get("id")
        if not isinstance(reaction_id, int):
            continue
        user_obj = r.get("user") or {}
        user = (user_obj.get("login") or "").strip().lower()
        if not user or user in blacklist:
            continue
        all_reaction_ids.append(reaction_id)
        if content == "+1":
            user_up_ids.setdefault(user, []).append(reaction_id)
        else:
            user_down_ids.setdefault(user, []).append(reaction_id)
    conflict_users = sorted(set(user_up_ids.keys()) & set(user_down_ids.keys()))
    raw_up = sum(1 for u in user_up_ids if u not in conflict_users)
    raw_down = sum(1 for u in user_down_ids if u not in conflict_users)
    return {
        "raw_up": raw_up,
        "raw_down": raw_down,
        "all_reaction_ids": sorted(all_reaction_ids),
        "conflict_users": conflict_users,
        "non_conflict_pool": [u for u in set(user_up_ids.keys()) | set(user_down_ids.keys())
                              if u not in conflict_users],
        "user_up_ids": user_up_ids,
        "user_down_ids": user_down_ids,
    }

# ---------------------------------------------------------------------------
# Account eligibility
# ---------------------------------------------------------------------------

def _parse_gh_ts(ts: str | None) -> datetime | None:
    if not ts:
        return None
    try:
        return datetime.fromisoformat(ts.replace("Z", "+00:00"))
    except (ValueError, TypeError):
        return None

def check_user_eligibility(
    login: str, policy: PolicyConfig, *, token: str, cache: dict[str, bool],
    now: datetime | None = None,
) -> bool:
    if login in cache:
        return cache[login]
    if now is None:
        now = datetime.now(timezone.utc)
    profile = _fetch_user_profile(login, token=token)
    if profile is None:
        cache[login] = False
        return False
    created_ts = _parse_gh_ts(profile.get("created_at"))
    if created_ts is None:
        cache[login] = False
        return False
    eligible = (now - created_ts).days >= policy.account_age_days
    cache[login] = eligible
    return eligible

# ---------------------------------------------------------------------------
# Anomaly detection
# ---------------------------------------------------------------------------

def detect_anomaly(
    downvote_ids: list[int], downvote_timestamps: list[datetime],
    policy: PolicyConfig, now: datetime | None = None,
) -> dict[str, Any]:
    if now is None:
        now = datetime.now(timezone.utc)
    window_start = now - timedelta(minutes=policy.raid_window_minutes)
    seven_days_ago = now - timedelta(days=7)
    recent_ids: list[int] = []
    recent_ts_list: list[datetime] = []
    all_7d_ids: list[int] = []
    for rid, ts in zip(downvote_ids, downvote_timestamps):
        if ts >= seven_days_ago:
            all_7d_ids.append(rid)
        if ts >= window_start:
            recent_ids.append(rid)
            recent_ts_list.append(ts)
    is_anomaly = False
    historical_avg = 0.0
    baseline_ratio = 0.0
    if policy.use_baseline:
        num_windows_7d = max(7 * 24 * 60 / policy.raid_window_minutes, 1.0)
        historical_avg = len(all_7d_ids) / num_windows_7d
        baseline_ratio = len(recent_ids) / max(historical_avg, 1.0)
        if len(recent_ids) > 20 and baseline_ratio > 5.0:
            is_anomaly = True
    else:
        is_anomaly = len(recent_ids) >= policy.raid_threshold
    # detected_at_bucket = raid_window-rounded now for stable event identity
    window_seconds = policy.raid_window_minutes * 60
    bucket_ts = int(now.timestamp() / window_seconds) * window_seconds
    detected_at_bucket = datetime.fromtimestamp(bucket_ts, tz=timezone.utc).isoformat()
    return {
        "is_anomaly": is_anomaly,
        "affected_reaction_ids": sorted(recent_ids),
        "count_in_window": len(recent_ids),
        "threshold": policy.raid_threshold,
        "window_minutes": policy.raid_window_minutes,
        "historical_avg": historical_avg,
        "baseline_ratio": baseline_ratio,
        "detected_at_bucket": detected_at_bucket,
    }

# ---------------------------------------------------------------------------
# State file I/O
# ---------------------------------------------------------------------------

def governance_state_semantic_view(state: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": state.get("schema_version", 1),
        "governance_repository": state.get("governance_repository"),
        "policy": state.get("policy"),
        "events": state.get("events", []),
    }

def load_vote_issues(path: Path) -> dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, json.JSONDecodeError):
        return {"schema_version": 1, "items": {}}

def load_governance_state(path: Path | None, *, governance_repo: str | None = None, policy: str | None = None) -> dict[str, Any]:
    if path is None:
        return {"schema_version": 1, "events": []}
    try:
        with path.open("r", encoding="utf-8") as fh:
            data = json.load(fh)
    except (OSError, json.JSONDecodeError):
        return {"schema_version": 1, "events": []}
    if not isinstance(data, dict):
        return {"schema_version": 1, "events": []}
    stored_repo = data.get("governance_repository")
    stored_policy = data.get("policy")
    if stored_repo and governance_repo and stored_repo != governance_repo:
        logger.warning("State governance_repository '%s' != expected '%s'; ignoring mismatched state.", stored_repo, governance_repo)
        return {"schema_version": 1, "events": []}
    if stored_policy and policy and stored_policy != policy:
        logger.warning("State policy '%s' != expected '%s'; ignoring mismatched state.", stored_policy, policy)
        return {"schema_version": 1, "events": []}
    if "events" not in data:
        data["events"] = []
    return data

def save_governance_state(path: Path, data: dict[str, Any]) -> bool:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("r", encoding="utf-8") as fh:
            existing = json.load(fh)
        if (
            isinstance(existing, dict)
            and governance_state_semantic_view(existing)
            == governance_state_semantic_view(data)
        ):
            logger.info("Governance state unchanged; preserving existing file.")
            return False
    except (OSError, json.JSONDecodeError):
        pass
    state = {**data, "generated_at": datetime.now(timezone.utc).isoformat()}
    tmp = path.with_suffix(path.suffix + ".tmp")
    with tmp.open("w", encoding="utf-8") as fh:
        json.dump(state, fh, indent=2)
    tmp.replace(path)
    logger.info("Governance state saved to %s", path)
    return True

def load_quarantine_decisions(path: Path) -> dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, json.JSONDecodeError):
        return {"schema_version": 1, "decisions": []}

# ---------------------------------------------------------------------------
# Discord embed builder
# ---------------------------------------------------------------------------

def build_discord_embed(*, mod_id: str, event_type: str, event_data: dict[str, Any],
                        policy: GovernancePolicy, mode: GovernanceMode) -> dict[str, Any] | None:
    if mode != GovernanceMode.MONITOR:
        return None
    if event_type == "vote_surge":
        color = 0xED4245
        title = f"Vote Anomaly Detected: {mod_id}"
        fields = [
            {"name": "Item ID", "value": mod_id, "inline": True},
            {"name": "Item Name", "value": event_data.get("item_name", ""), "inline": True},
            {"name": "Issue URL", "value": event_data.get("issue_url", ""), "inline": False},
            {"name": "Newly Quarantined", "value": str(event_data.get("newly_quarantined", 0)), "inline": True},
            {"name": "Total Quarantined", "value": str(event_data.get("total_quarantined", 0)), "inline": True},
            {"name": "Before Score", "value": event_data.get("before_score", "N/A"), "inline": True},
            {"name": "Users", "value": str(event_data.get("user_count", 0)), "inline": True},
            {"name": "Event ID", "value": event_data.get("event_id", ""), "inline": False},
            {"name": "Decision", "value": "Pending curator review", "inline": False},
        ]
    elif event_type == "resolved":
        color = 0xFEE75C
        title = f"Resolved: {mod_id}"
        fields = [
            {"name": "Item ID", "value": mod_id, "inline": True},
            {"name": "Item Name", "value": event_data.get("item_name", ""), "inline": True},
            {"name": "Issue URL", "value": event_data.get("issue_url", ""), "inline": False},
            {"name": "Event ID", "value": event_data.get("event_id", ""), "inline": False},
            {"name": "Decision", "value": event_data.get("decision", "unknown"), "inline": False},
        ]
    else:
        return None
    fields.append({"name": "Policy", "value": policy.value, "inline": True})
    embed = {
        "title": title, "color": color,
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "fields": fields,
        "footer": {"text": "Agora Governance"},
    }
    return {"username": "Agora Governance", "embeds": [embed]}

def _extract_event_id_from_embed(embed: dict[str, Any]) -> str | None:
    for emb in embed.get("embeds", []):
        for field in emb.get("fields", []):
            if field.get("name") == "Event ID":
                return field.get("value") or None
    return None

def _post_discord(webhook_url: str, embed: dict[str, Any]) -> bool:
    import requests
    event_id = _extract_event_id_from_embed(embed) or "unknown"
    try:
        resp = requests.post(webhook_url, json=embed, timeout=15)
        if resp.status_code not in (200, 204):
            logger.warning(
                "Discord webhook returned HTTP %d for event %s",
                resp.status_code, event_id,
            )
            return False
        return True
    except Exception as exc:
        logger.warning(
            "Discord webhook POST failed for event %s (HTTP status unavailable): %s",
            event_id,
            exc,
        )
        return False

# ---------------------------------------------------------------------------
# Pipeline
# ---------------------------------------------------------------------------

def run_governance_pipeline(
    items: list[dict[str, Any]],
    *,
    mode: GovernanceMode, policy: GovernancePolicy,
    governance_repo: str | None, token: str | None, blacklist: set[str],
    vote_issues_path: Path,
    governance_state_in_path: Path | None,
    governance_state_out_path: Path,
    quarantine_decisions_path: Path,
    discord_webhook_url: str | None,
) -> dict[str, Any]:
    if mode == GovernanceMode.OFF:
        logger.info("Governance mode=off.")
        return {}
    if not token:
        logger.warning("GITHUB_TOKEN not set.")
        return {}
    if not governance_repo:
        logger.warning("Governance repo not resolved.")
        return {}
    owner, _, repo_name = governance_repo.partition("/")
    if not owner or not repo_name:
        logger.warning("Governance repo '%s' invalid.", governance_repo)
        return {}

    vi = load_vote_issues(vote_issues_path)
    item_issue_map: dict[str, int] = {}
    for item_id, entry in vi.get("items", {}).items():
        issue_n = entry.get("issue_number") if isinstance(entry, dict) else entry
        if isinstance(issue_n, int):
            item_issue_map[item_id.lower()] = issue_n

    # Build all known registry item IDs for independent review extraction.
    all_registry_ids: set[str] = set()
    immune_ids: set[str] = set()
    for it in items:
        iid = it.get("id", "").lower()
        if iid:
            all_registry_ids.add(iid)
            gov_block = it.get("governance") or {}
            if gov_block.get("immune") is True:
                immune_ids.add(iid)

    try:
        all_issues = _fetch_issues(owner, repo_name, token=token)
    except RuntimeError as exc:
        logger.warning("Failed to fetch issues: %s", exc)
        return {}

    # Extract reviews for ALL registry items, not just vote-issue-mapped ones.
    known_mod_id_set = all_registry_ids
    reviews_by_item = extract_reviews(all_issues, known_mod_id_set)
    for item_reviews in reviews_by_item.values():
        for review in item_reviews:
            review["issue_url"] = (
                f"https://github.com/{governance_repo}/issues/{review['issue_number']}"
            )
    known_issue_nums = set(item_issue_map.values())

    # Load previous state and curator decisions
    state_data = load_governance_state(governance_state_in_path, governance_repo=governance_repo, policy=policy.value)
    prev_events: list[dict[str, Any]] = state_data.get("events", [])
    prev_event_map: dict[str, dict[str, Any]] = {}
    for ev in prev_events:
        eid = ev.get("event_id", "")
        if eid:
            prev_event_map[eid] = ev

    qd = load_quarantine_decisions(quarantine_decisions_path)
    curator_decisions: dict[str, str] = {}
    for d in qd.get("decisions", []):
        eid = d.get("event_id", "")
        st = d.get("status", "")
        if eid and st:
            curator_decisions[eid] = st

    stored_status_by_id = {
        ev.get("event_id", ""): ev.get("status", "pending") for ev in prev_events
    }

    # Override prev event statuses with curator decisions
    for ev in prev_events:
        eid = ev.get("event_id", "")
        if eid in curator_decisions:
            ev["status"] = curator_decisions[eid]

    # Build item name lookup
    item_name_map: dict[str, str] = {}
    for it in items:
        iid = it.get("id", "").lower()
        if iid:
            item_name_map[iid] = it.get("name", "")

    policy_cfg = build_policy_config(policy)

    results: dict[str, Any] = {}
    new_events: list[dict[str, Any]] = []
    discord_notified: list[str] = []

    # First pass: emit review-only results for ALL registry items
    for item_id in all_registry_ids:
        reviews = reviews_by_item.get(item_id, [])
        review_count = len(reviews)
        unique_reviewers = len({r["author"] for r in reviews})
        vote_issue_number = item_issue_map.get(item_id, 0)
        vote_issue_url = f"https://github.com/{governance_repo}/issues/{vote_issue_number}" if vote_issue_number else ""
        is_immune = item_id in immune_ids

        results[item_id] = {
            "item_id": item_id,
            "item_name": item_name_map.get(item_id, item_id),
            "vote_issue_number": vote_issue_number,
            "vote_issue_url": vote_issue_url,
            "raw_upvotes": 0, "raw_downvotes": 0,
            "counted_upvotes": 0, "counted_downvotes": 0,
            "quarantined_upvotes": 0, "quarantined_downvotes": 0,
            "conflicted_users": [],
            "status_reason": "normal",
            "registry_status": "active",
            "review_count": review_count,
            "unique_reviewers": unique_reviewers,
            "event_id": None,
            "affected_reaction_ids": [],
            "reviews": reviews,
            "eligible_up": 0, "eligible_down": 0,
            "anomaly": False, "anomaly_count": 0,
            "is_immune": is_immune,
        }

    # Second pass: vote/anomaly processing only for mapped (non-immune) items
    for item_id, issue_number in item_issue_map.items():
        if item_id not in all_registry_ids:
            continue
        if item_id in immune_ids:
            continue
        if issue_number not in known_issue_nums:
            continue
        try:
            reactions = _fetch_reactions(owner, repo_name, issue_number, token=token)
        except RuntimeError:
            reactions = []

        tally = tally_item_votes(reactions, blacklist)
        raw_up = tally["raw_up"]
        raw_down = tally["raw_down"]
        all_rids = tally["all_reaction_ids"]
        conflict_users = tally["conflict_users"]

        downvote_rids: list[int] = []
        downvote_ts: list[datetime] = []
        for r in reactions:
            if r.get("content") != "-1":
                continue
            rid = r.get("id")
            if not isinstance(rid, int):
                continue
            u_obj = r.get("user") or {}
            user = (u_obj.get("login") or "").strip().lower()
            if not user or user in blacklist:
                continue
            if user in tally["conflict_users"]:
                continue
            ts = _parse_gh_ts(r.get("created_at"))
            if ts:
                downvote_rids.append(rid)
                downvote_ts.append(ts)

        rejected_rids: set[int] = set()
        for previous in prev_events:
            if previous.get("item_id") == item_id and previous.get("status") == "rejected":
                rejected_rids.update(previous.get("affected_reactions") or [])
        anomaly_pairs = [
            (rid, timestamp)
            for rid, timestamp in zip(downvote_rids, downvote_ts)
            if rid not in rejected_rids
        ]
        anomaly = detect_anomaly(
            [pair[0] for pair in anomaly_pairs],
            [pair[1] for pair in anomaly_pairs],
            policy_cfg,
        )

        # Eligible counts (all non-conflict users with sufficient account age)
        eligible_cache: dict[str, bool] = {}
        eligible_up = 0
        eligible_down = 0
        for user in tally["non_conflict_pool"]:
            if not check_user_eligibility(user, policy_cfg, token=token, cache=eligible_cache):
                continue
            if user in tally["user_up_ids"]:
                eligible_up += 1
            if user in tally["user_down_ids"]:
                eligible_down += 1

        anomaly_rids = set(anomaly["affected_reaction_ids"])

        # Determine which previous events are still unresolved (pending/rejected) for this item
        excluded_event_ids: set[str] = set()
        pending_event_ids: set[str] = set()
        for ev in prev_events:
            if ev.get("item_id") != item_id:
                continue
            eid = ev.get("event_id", "")
            e_status = ev.get("status", "pending")
            local_status = curator_decisions.get(eid, e_status)
            if local_status in ("pending", "rejected"):
                excluded_event_ids.add(eid)
            if local_status == "pending":
                pending_event_ids.add(eid)

        excluded_rids: set[int] = set()
        for ev_entry in prev_events:
            eid = ev_entry.get("event_id", "")
            if eid not in excluded_event_ids:
                continue
            affected = ev_entry.get("affected_reactions", [])
            if isinstance(affected, list):
                excluded_rids.update(int(x) for x in affected if str(x).isdigit())

        # Reuse one unresolved event per item and canonical issue so growth
        # merges into the original event instead of generating overlapping IDs.
        active_event = None
        for previous in reversed(prev_events):
            if previous.get("item_id") != item_id:
                continue
            try:
                previous_details = json.loads(previous.get("details_json") or "{}")
            except (TypeError, json.JSONDecodeError):
                previous_details = {}
            if previous_details.get("issue_number") != issue_number:
                continue
            previous_status = previous.get("status", "pending")
            if previous_status == "pending":
                active_event = previous
                break

        # New anomaly IDs also excluded (unless the matching event was accepted).
        stable_id = None
        if anomaly["is_anomaly"]:
            stable_id = (
                active_event.get("event_id")
                if active_event is not None
                else make_stable_event_id(item_id, issue_number, anomaly["detected_at_bucket"])
            )
            if stable_id not in curator_decisions or curator_decisions[stable_id] != "accepted":
                excluded_rids.update(anomaly_rids)

        counted_up = 0
        counted_down = 0
        quarantined_up = 0
        quarantined_down = 0

        for user in tally["non_conflict_pool"]:
            if not check_user_eligibility(user, policy_cfg, token=token, cache=eligible_cache):
                continue
            is_up = user in tally["user_up_ids"]
            is_down = user in tally["user_down_ids"]
            rid_list = tally["user_up_ids"].get(user, []) + tally["user_down_ids"].get(user, [])
            any_excluded = any(rid in excluded_rids for rid in rid_list)
            if is_up:
                if any_excluded:
                    quarantined_up += 1
                else:
                    counted_up += 1
            if is_down:
                if any_excluded:
                    quarantined_down += 1
                else:
                    counted_down += 1

        reviews = reviews_by_item.get(item_id, [])
        review_count = len(reviews)
        unique_reviewers = len({r["author"] for r in reviews})

        vote_issue_url = f"https://github.com/{governance_repo}/issues/{issue_number}"

        # Stable event identity: item+issue+anomaly-start bucket
        detected_now = datetime.now(timezone.utc)
        anomaly_detected_bucket = anomaly.get("detected_at_bucket", detected_now.isoformat())
        stable_event_id = None
        event_type = None
        event_status = "pending"
        if anomaly["is_anomaly"]:
            stable_event_id = stable_id or make_stable_event_id(
                item_id, issue_number, anomaly_detected_bucket
            )
            prev_stable = prev_event_map.get(stable_event_id)
            if prev_stable is None:
                event_type = "vote_surge"
                event_status = "pending"
            else:
                p_status = curator_decisions.get(stable_event_id, prev_stable.get("status", "pending"))
                event_status = p_status
                if p_status == "pending":
                    event_type = "vote_surge"
                elif p_status == "rejected":
                    event_type = "rejected"
                # accepted: no new event

        # Determine registry_status (separate from status_reason)
        has_unresolved = bool(pending_event_ids) or (
            anomaly["is_anomaly"] and event_status == "pending"
        )
        registry_status = "under_review" if has_unresolved else "active"
        status_reason = "pending_vote_quarantine" if has_unresolved else "normal"

        # Compute velocity from current/historical counts
        velocity = 0.0
        total_recent = counted_up + counted_down
        # If we have any counted votes, derive velocity from current vs historical distribution
        # Use upvote/downvote timestamps from the raw reactions for velocity
        up_ts_all: list[datetime] = []
        down_ts_all: list[datetime] = []
        for r in reactions:
            if r.get("content") == "+1":
                ts = _parse_gh_ts(r.get("created_at"))
                if ts: up_ts_all.append(ts)
            elif r.get("content") == "-1":
                ts = _parse_gh_ts(r.get("created_at"))
                if ts: down_ts_all.append(ts)
        if up_ts_all or down_ts_all:
            seven_d_ago = detected_now - timedelta(days=7)
            six_h_ago = detected_now - timedelta(hours=6)
            up_7d = [t for t in up_ts_all if seven_d_ago <= t <= detected_now]
            down_7d = [t for t in down_ts_all if seven_d_ago <= t <= detected_now]
            up_6h = [t for t in up_ts_all if six_h_ago <= t <= detected_now]
            down_6h = [t for t in down_ts_all if six_h_ago <= t <= detected_now]
            total_7d = len(up_7d) + len(down_7d)
            historical_avg_per_6h = total_7d / 28.0
            recent_6h_total = len(up_6h) + len(down_6h)
            if historical_avg_per_6h < 0.5:
                velocity = (recent_6h_total / 0.5) - 1.0
            else:
                velocity = (recent_6h_total - historical_avg_per_6h) / historical_avg_per_6h
            velocity = max(-10.0, min(10.0, velocity))

        # Build new event entry
        if event_type == "vote_surge":
            event_detected_at = prev_event_map.get(stable_event_id, {}).get(
                "detected_at", detected_now.isoformat()
            )
            event_affected_reactions = sorted(
                set(prev_event_map.get(stable_event_id, {}).get("affected_reactions") or [])
                | anomaly_rids
            )
            ev_entry = {
                "event_id": stable_event_id,
                "item_id": item_id,
                "event_type": "vote_surge",
                "status": event_status,
                "detected_at": event_detected_at,
                "affected_reactions": event_affected_reactions,
                "details_json": json.dumps({
                    "item_id": item_id,
                    "issue_number": issue_number,
                    "threshold": anomaly["threshold"],
                    "window_minutes": anomaly["window_minutes"],
                    "historical_avg": anomaly.get("historical_avg", 0),
                    "baseline_ratio": anomaly.get("baseline_ratio", 0),
                    "count_in_window": anomaly["count_in_window"],
                    "reaction_ids": list(anomaly_rids),
                    "raw_up": raw_up,
                    "raw_down": raw_down,
                    "eligible_up": eligible_up,
                    "eligible_down": eligible_down,
                    "counted_up": counted_up,
                    "counted_down": counted_down,
                    "quarantined_up": quarantined_up,
                    "quarantined_down": quarantined_down,
                    "conflict_users": tally["conflict_users"],
                }, separators=(",", ":")),
            }
            new_events.append(ev_entry)

        previous_affected_for_alert: set[int] = set()
        if stable_event_id and stable_event_id in prev_event_map:
            previous_affected_for_alert = set(
                prev_event_map[stable_event_id].get("affected_reactions") or []
            )

        # Update existing stable event if more reactions came in (growth merge)
        if stable_event_id and prev_event_map.get(stable_event_id) and anomaly["is_anomaly"]:
            prev_ev = prev_event_map[stable_event_id]
            prev_affected = set(prev_ev.get("affected_reactions") or [])
            new_affected = set(anomaly_rids)
            merged_affected = sorted(prev_affected | new_affected)
            if merged_affected != sorted(prev_affected):
                # Update in prev_events list for state persistence
                for pe in prev_events:
                    if pe.get("event_id") == stable_event_id:
                        pe["affected_reactions"] = merged_affected
                        break

        # Discord alerts: only new/grown/resolved
        if discord_webhook_url and mode == GovernanceMode.MONITOR:
            item_name = item_name_map.get(item_id, item_id)
            is_new = anomaly["is_anomaly"] and stable_event_id and stable_event_id not in prev_event_map
            is_grown = False
            if anomaly["is_anomaly"] and stable_event_id in prev_event_map:
                is_grown = bool(anomaly_rids - previous_affected_for_alert)
            is_resolved = any(
                stored_status_by_id.get(event_id) == "pending"
                and curator_decisions.get(event_id) in ("accepted", "rejected")
                and prev_event_map.get(event_id, {}).get("item_id") == item_id
                for event_id in stored_status_by_id
            )

            if is_new:
                discord_data = {
                    "item_name": item_name, "issue_url": vote_issue_url,
                    "newly_quarantined": len(anomaly_rids),
                    "total_quarantined": quarantined_up + quarantined_down,
                    "before_score": f"{eligible_up - eligible_down}",
                    "user_count": len(set(
                        u for u in tally["non_conflict_pool"]
                        if check_user_eligibility(u, policy_cfg, token=token, cache=eligible_cache)
                    )),
                    "event_id": stable_event_id,
                }
                embed = build_discord_embed(
                    mod_id=item_id, event_type="vote_surge",
                    event_data=discord_data, policy=policy, mode=mode,
                )
                if embed and _post_discord(discord_webhook_url, embed):
                    discord_notified.append(item_id)
            elif is_grown:
                discord_data = {
                    "item_name": item_name, "issue_url": vote_issue_url,
                    "newly_quarantined": len(anomaly_rids - previous_affected_for_alert),
                    "total_quarantined": quarantined_up + quarantined_down,
                    "before_score": f"{eligible_up - eligible_down}",
                    "user_count": len(set(
                        u for u in tally["non_conflict_pool"]
                        if check_user_eligibility(u, policy_cfg, token=token, cache=eligible_cache)
                    )),
                    "event_id": stable_event_id,
                }
                embed = build_discord_embed(
                    mod_id=item_id, event_type="vote_surge",
                    event_data=discord_data, policy=policy, mode=mode,
                )
                if embed and _post_discord(discord_webhook_url, embed):
                    discord_notified.append(item_id)
            elif is_resolved:
                resolved_event_id = next(
                    (
                        event_id
                        for event_id, old_status in stored_status_by_id.items()
                        if old_status == "pending"
                        and curator_decisions.get(event_id) in ("accepted", "rejected")
                        and prev_event_map.get(event_id, {}).get("item_id") == item_id
                    ),
                    "",
                )
                discord_data = {
                    "item_name": item_name, "issue_url": vote_issue_url,
                    "event_id": resolved_event_id,
                    "decision": curator_decisions.get(resolved_event_id, "resolved"),
                }
                embed = build_discord_embed(
                    mod_id=item_id, event_type="resolved",
                    event_data=discord_data, policy=policy, mode=mode,
                )
                if embed and _post_discord(discord_webhook_url, embed):
                    discord_notified.append(item_id)

        results[item_id] = {
            "item_id": item_id,
            "item_name": item_name_map.get(item_id, item_id),
            "vote_issue_number": issue_number,
            "vote_issue_url": vote_issue_url,
            "raw_upvotes": raw_up,
            "raw_downvotes": raw_down,
            "counted_upvotes": counted_up,
            "counted_downvotes": counted_down,
            "quarantined_upvotes": quarantined_up,
            "quarantined_downvotes": quarantined_down,
            "conflicted_users": conflict_users,
            "status_reason": status_reason,
            "registry_status": registry_status,
            "review_count": review_count,
            "unique_reviewers": unique_reviewers,
            "event_id": stable_event_id if anomaly["is_anomaly"] else None,
            "affected_reaction_ids": sorted(anomaly_rids),
            "reviews": reviews,
            "eligible_up": eligible_up,
            "eligible_down": eligible_down,
            "anomaly": anomaly["is_anomaly"],
            "anomaly_count": anomaly["count_in_window"],
            "is_immune": False,
            "velocity": velocity,
        }

    # Build new state with all events, including previous unseen ones
    # Merge new events into previous (updating existing stable events)
    merged_events = list(prev_events)
    for ne in new_events:
        eid = ne["event_id"]
        found = False
        for i, pe in enumerate(merged_events):
            if pe.get("event_id") == eid:
                merged_events[i] = ne
                found = True
                break
        if not found:
            merged_events.append(ne)

    new_state = {
        "schema_version": 1,
        "governance_repository": governance_repo,
        "policy": policy.value,
        "events": merged_events,
    }

    if mode == GovernanceMode.MONITOR:
        save_governance_state(governance_state_out_path, new_state)

    results["_meta"] = {
        "mode": mode.value, "policy": policy.value, "repo": governance_repo,
        "total_items_mapped": len(item_issue_map),
        "total_items_in_registry": len(all_registry_ids),
        "total_new_events": len(new_events),
        "total_prev_events": len(prev_events),
        "discord_notified": len(discord_notified),
        "events": merged_events,
    }
    return results

# ---------------------------------------------------------------------------
# DB enrichment (exact governance_summary / governance_events)
# ---------------------------------------------------------------------------

def enrich_governance_summary(conn, governance_results: dict[str, Any]) -> None:
    if not governance_results:
        return
    cursor = conn.cursor()
    for item_id, data in governance_results.items():
        if item_id.startswith("_"):
            continue
        cursor.execute(
            """
            INSERT INTO governance_summary (
                item_id, vote_issue_number, vote_issue_url,
                raw_upvotes, raw_downvotes, counted_upvotes, counted_downvotes,
                quarantined_upvotes, quarantined_downvotes,
                conflicted_users, status_reason, compiled_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(item_id) DO UPDATE SET
                vote_issue_number = excluded.vote_issue_number,
                vote_issue_url = excluded.vote_issue_url,
                raw_upvotes = excluded.raw_upvotes,
                raw_downvotes = excluded.raw_downvotes,
                counted_upvotes = excluded.counted_upvotes,
                counted_downvotes = excluded.counted_downvotes,
                quarantined_upvotes = excluded.quarantined_upvotes,
                quarantined_downvotes = excluded.quarantined_downvotes,
                conflicted_users = excluded.conflicted_users,
                status_reason = excluded.status_reason,
                compiled_at = excluded.compiled_at
            """,
            (
                item_id,
                data.get("vote_issue_number", 0),
                data.get("vote_issue_url", ""),
                data.get("raw_upvotes", 0),
                data.get("raw_downvotes", 0),
                data.get("counted_upvotes", 0),
                data.get("counted_downvotes", 0),
                data.get("quarantined_upvotes", 0),
                data.get("quarantined_downvotes", 0),
                len(data.get("conflicted_users", [])),
                data.get("status_reason", ""),
                datetime.now(timezone.utc).isoformat(),
            ),
        )

def enrich_governance_events(conn, governance_results: dict[str, Any]) -> None:
    if not governance_results:
        return
    cursor = conn.cursor()
    all_events = governance_results.get("_meta", {}).get("events", [])
    for ev in all_events:
        cursor.execute(
            """
            INSERT INTO governance_events (
                event_id, item_id, event_type, status,
                detected_at, affected_reactions, details_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(event_id) DO UPDATE SET
                status = excluded.status,
                detected_at = excluded.detected_at,
                affected_reactions = excluded.affected_reactions,
                details_json = excluded.details_json
            """,
            (
                ev["event_id"], ev["item_id"], ev["event_type"], ev["status"],
                ev["detected_at"], len(ev.get("affected_reactions", [])), ev["details_json"],
            ),
        )

def enrich_registry_item_scores(item: dict[str, Any], governance_results: dict[str, Any]) -> None:
    item_id = item.get("id", "").lower()
    if not item_id:
        return
    data = governance_results.get(item_id)
    if not data:
        return
    item["_governance_counted_upvotes"] = data.get("counted_upvotes", 0)
    item["_governance_counted_downvotes"] = data.get("counted_downvotes", 0)
    item["_governance_net_score"] = data.get("counted_upvotes", 0) - data.get("counted_downvotes", 0)
    item["_governance_review_count"] = data.get("review_count", 0)
    item["_governance_status_reason"] = data.get("status_reason", "")
    item["_governance_registry_status"] = data.get("registry_status", "active")
    item["_governance_raw_upvotes"] = data.get("raw_upvotes", 0)
    item["_governance_raw_downvotes"] = data.get("raw_downvotes", 0)
    item["_governance_quarantined_upvotes"] = data.get("quarantined_upvotes", 0)
    item["_governance_quarantined_downvotes"] = data.get("quarantined_downvotes", 0)
    item["_governance_vote_issue_url"] = data.get("vote_issue_url", "")
    item["_governance_velocity"] = data.get("velocity", 0.0)
    reviews = data.get("reviews", [])
    if reviews:
        enriched = []
        for r in reviews:
            enriched.append({
                "author": r.get("author", ""),
                "text": r.get("technical_review", ""),
                "item_version": r.get("item_version", ""),
                "minecraft_version": r.get("minecraft_version", ""),
                "loader": r.get("loader", ""),
                "relationship": r.get("relationship", ""),
                "focus": r.get("focus", ""),
                "evidence": r.get("evidence", ""),
                "limitations": r.get("limitations", ""),
                "issue_url": r.get("issue_url", ""),
                "issue_number": r.get("issue_number", 0),
                "created_at": r.get("created_at", ""),
            })
        item["_governance_top_reviews"] = enriched[:10]

def enrich_governance_tables(conn, governance_results: dict[str, Any]) -> None:
    enrich_governance_summary(conn, governance_results)
    enrich_governance_events(conn, governance_results)
