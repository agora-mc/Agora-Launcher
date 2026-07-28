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

def build_policy_config(policy: GovernancePolicy) -> PolicyConfig:
    if policy == GovernancePolicy.SANDBOX:
        return PolicyConfig(
            account_age_days=SANDBOX_ACCOUNT_AGE_DAYS,
            raid_threshold=SANDBOX_RAID_THRESHOLD,
            raid_window_minutes=SANDBOX_RAID_WINDOW_MINUTES,
        )
    return PolicyConfig()

# ---------------------------------------------------------------------------
# Digest / event_id
# ---------------------------------------------------------------------------

def make_event_id(item_id: str, issue_number: int, sorted_ids: list[int]) -> str:
    raw = f"{item_id}:{issue_number}:{','.join(str(i) for i in sorted_ids)}"
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
    recent_ids: list[int] = []
    for rid, ts in zip(downvote_ids, downvote_timestamps):
        if ts >= window_start:
            recent_ids.append(rid)
    is_anomaly = len(recent_ids) >= policy.raid_threshold
    return {
        "is_anomaly": is_anomaly,
        "affected_reaction_ids": sorted(recent_ids),
        "count_in_window": len(recent_ids),
        "threshold": policy.raid_threshold,
        "window_minutes": policy.raid_window_minutes,
    }

# ---------------------------------------------------------------------------
# State file I/O
# ---------------------------------------------------------------------------

def load_vote_issues(path: Path) -> dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, json.JSONDecodeError):
        return {"schema_version": 1, "items": {}}

def load_governance_state(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {"schema_version": 1, "events": []}
    try:
        with path.open("r", encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, json.JSONDecodeError):
        return {"schema_version": 1, "events": []}

def save_governance_state(path: Path, data: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    with tmp.open("w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2)
    tmp.replace(path)

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
    elif event_type == "new_review":
        color = 0x57F287
        title = f"New Review: {mod_id}"
        fields = [
            {"name": "Item ID", "value": mod_id, "inline": True},
            {"name": "Item Name", "value": event_data.get("item_name", ""), "inline": True},
            {"name": "Issue URL", "value": event_data.get("issue_url", ""), "inline": False},
            {"name": "Review Count", "value": str(event_data.get("review_count", 0)), "inline": True},
        ]
    elif event_type == "resolved":
        color = 0xFEE75C
        title = f"Resolved: {mod_id}"
        fields = [
            {"name": "Item ID", "value": mod_id, "inline": True},
            {"name": "Item Name", "value": event_data.get("item_name", ""), "inline": True},
            {"name": "Issue URL", "value": event_data.get("issue_url", ""), "inline": False},
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

def _post_discord(webhook_url: str, embed: dict[str, Any]) -> None:
    import requests
    try:
        resp = requests.post(webhook_url, json=embed, timeout=15)
        if resp.status_code not in (200, 204):
            logger.warning("Discord webhook returned HTTP %d", resp.status_code)
    except Exception as exc:
        logger.warning("Discord webhook POST failed: %s", exc)

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
    if not item_issue_map:
        logger.info("vote_issues.json has no mapped items.")
        return {}

    try:
        all_issues = _fetch_issues(owner, repo_name, token=token)
    except RuntimeError as exc:
        logger.warning("Failed to fetch issues: %s", exc)
        return {}

    known_mod_id_set = set(item_issue_map.keys())
    reviews_by_item = extract_reviews(all_issues, known_mod_id_set)
    for item_reviews in reviews_by_item.values():
        for review in item_reviews:
            review["issue_url"] = (
                f"https://github.com/{governance_repo}/issues/{review['issue_number']}"
            )
    known_issue_nums = set(item_issue_map.values())

    # Load previous state and curator decisions
    state_data = load_governance_state(governance_state_in_path)
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

    for item_id, issue_number in item_issue_map.items():
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

        anomaly = detect_anomaly(downvote_rids, downvote_ts, policy_cfg)

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

        # Quarantined counts: which of the eligible reaction IDs are under pending/rejected events
        quarantined_rids: set[int] = set()
        for ev in prev_events:
            if ev.get("item_id") != item_id:
                continue
            eid = ev.get("event_id", "")
            e_status = ev.get("status", "pending")
            if e_status in ("pending", "rejected") and e_status != "accepted":
                affected = ev.get("affected_reactions", [])
                if isinstance(affected, list):
                    quarantined_rids.update(int(x) for x in affected if isinstance(x, (int, str)) and str(x).isdigit())
        # Also apply from new anomaly if one is detected
        anomaly_rids = set(anomaly["affected_reaction_ids"])

        # Quarantined up/down: reaction IDs that are in quarantined_rids OR in anomaly_rids
        # (but NOT if the event was accepted by curator)
        excluded_rids: set[int] = set()
        for ev_entry in prev_events:
            eid = ev_entry.get("event_id", "")
            e_status = ev_entry.get("status", "pending")
            # Only curator "accepted" restores; pending/rejected exclude
            local_status = curator_decisions.get(eid, e_status)
            if local_status == "accepted":
                continue
            affected = ev_entry.get("affected_reactions", [])
            if isinstance(affected, list):
                excluded_rids.update(int(x) for x in affected if str(x).isdigit())

        # New anomaly IDs also excluded (unless matching an accepted event)
        if anomaly["is_anomaly"]:
            new_anomaly_event_id = make_event_id(item_id, issue_number, sorted(anomaly_rids))
            if new_anomaly_event_id not in curator_decisions or curator_decisions[new_anomaly_event_id] != "accepted":
                excluded_rids.update(anomaly_rids)

        counted_up = 0
        counted_down = 0
        quarantined_up = 0
        quarantined_down = 0

        # For each eligible user, check if their reaction IDs are excluded
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

        # Decide event creation
        anomaly_event_id = make_event_id(item_id, issue_number, sorted(list(anomaly_rids)))
        all_current_rids = sorted(all_rids)

        prev_anomaly_event = prev_event_map.get(anomaly_event_id)

        event_type = None
        status = "pending"

        if anomaly["is_anomaly"]:
            if prev_anomaly_event is None:
                event_type = "vote_surge"
                status = "pending"
            else:
                # Pre-existing event: status is from curator or previous state
                p_status = curator_decisions.get(anomaly_event_id, prev_anomaly_event.get("status", "pending"))
                status = p_status
                if p_status == "pending":
                    event_type = "vote_surge"
                elif p_status == "accepted":
                    pass  # already resolved
                elif p_status == "rejected":
                    event_type = "rejected"
        else:
            # No anomaly -- keep previous status if existed, but don't create new event
            if prev_anomaly_event is not None:
                p_status = curator_decisions.get(anomaly_event_id, prev_anomaly_event.get("status", "pending"))
                if p_status == "pending":
                    status = "pending"
                elif p_status == "accepted":
                    status = "accepted"

        if event_type == "vote_surge":
            ev_entry = {
                "event_id": anomaly_event_id,
                "item_id": item_id,
                "event_type": "vote_surge",
                "status": status,
                "detected_at": datetime.now(timezone.utc).isoformat(),
                "affected_reactions": sorted(anomaly_rids),
                "details_json": json.dumps({
                    "item_id": item_id,
                    "issue_number": issue_number,
                    "threshold": anomaly["threshold"],
                    "window_minutes": anomaly["window_minutes"],
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

        # Discord notifications
        if discord_webhook_url and mode == GovernanceMode.MONITOR:
            item_name = item_name_map.get(item_id, item_id)
            is_new = anomaly["is_anomaly"] and prev_anomaly_event is None
            is_grown = False
            if prev_anomaly_event is not None:
                prev_affected = prev_anomaly_event.get("affected_reactions", [])
                if len(anomaly_rids) > len(prev_affected):
                    is_grown = True
            is_resolved = False
            if prev_anomaly_event is not None and event_type is None:
                prev_st = curator_decisions.get(anomaly_event_id, prev_anomaly_event.get("status", "pending"))
                if prev_st == "pending" and status == "accepted":
                    is_resolved = True

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
                    "event_id": anomaly_event_id,
                }
                embed = build_discord_embed(
                    mod_id=item_id, event_type="vote_surge",
                    event_data=discord_data, policy=policy, mode=mode,
                )
                if embed:
                    _post_discord(discord_webhook_url, embed)
                    discord_notified.append(item_id)
            elif is_grown:
                discord_data = {
                    "item_name": item_name, "issue_url": vote_issue_url,
                    "newly_quarantined": len(anomaly_rids) - len(prev_anomaly_event.get("affected_reactions", [])),
                    "total_quarantined": quarantined_up + quarantined_down,
                    "before_score": f"{eligible_up - eligible_down}",
                    "user_count": len(set(
                        u for u in tally["non_conflict_pool"]
                        if check_user_eligibility(u, policy_cfg, token=token, cache=eligible_cache)
                    )),
                    "event_id": anomaly_event_id,
                }
                embed = build_discord_embed(
                    mod_id=item_id, event_type="vote_surge",
                    event_data=discord_data, policy=policy, mode=mode,
                )
                if embed:
                    _post_discord(discord_webhook_url, embed)
                    discord_notified.append(item_id)

        if review_count > 0 and (item_id not in discord_notified) and prev_anomaly_event is None and not anomaly["is_anomaly"]:
            if discord_webhook_url and mode == GovernanceMode.MONITOR:
                discord_data = {
                    "item_name": item_name, "issue_url": vote_issue_url,
                    "review_count": review_count,
                }
                embed = build_discord_embed(
                    mod_id=item_id, event_type="new_review",
                    event_data=discord_data, policy=policy, mode=mode,
                )
                if embed:
                    _post_discord(discord_webhook_url, embed)
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
            "status_reason": "vote_surge" if anomaly["is_anomaly"] else "normal",
            "review_count": review_count,
            "unique_reviewers": unique_reviewers,
            "event_id": anomaly_event_id if anomaly["is_anomaly"] else None,
            "affected_reaction_ids": sorted(anomaly_rids),
            "reviews": reviews,
            "eligible_up": eligible_up,
            "eligible_down": eligible_down,
            "anomaly": anomaly["is_anomaly"],
            "anomaly_count": anomaly["count_in_window"],
        }

    new_state_events = prev_events + new_events
    new_state = {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "events": new_state_events,
    }

    if mode == GovernanceMode.MONITOR:
        save_governance_state(governance_state_out_path, new_state)

    results["_meta"] = {
        "mode": mode.value, "policy": policy.value, "repo": governance_repo,
        "total_items_mapped": len(item_issue_map),
        "total_new_events": len(new_events),
        "total_prev_events": len(prev_events),
        "discord_notified": len(discord_notified),
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
    meta = governance_results.get("_meta", {})
    cursor = conn.cursor()
    for item_id, data in governance_results.items():
        if item_id.startswith("_") or data.get("event_id") is None:
            continue
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
                data["event_id"],
                item_id,
                "vote_surge",
                "pending",
                datetime.now(timezone.utc).isoformat(),
                len(data.get("affected_reaction_ids", [])),
                json.dumps({k: v for k, v in data.items()
                           if k not in ("reviews", "event_id")}, separators=(",", ":")),
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
    item["_governance_status"] = data.get("status_reason", "")
    item["_governance_raw_upvotes"] = data.get("raw_upvotes", 0)
    item["_governance_raw_downvotes"] = data.get("raw_downvotes", 0)
    item["_governance_quarantined_upvotes"] = data.get("quarantined_upvotes", 0)
    item["_governance_quarantined_downvotes"] = data.get("quarantined_downvotes", 0)
    item["_governance_vote_issue_url"] = data.get("vote_issue_url", "")
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
