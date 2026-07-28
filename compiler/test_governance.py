#!/usr/bin/env python3
from __future__ import annotations

"""Unit tests for compiler/governance.py — all network mocked."""

import json
import os
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

import governance as gov

NOW = datetime(2026, 7, 27, 12, 0, 0, tzinfo=timezone.utc)

def _set_age(days_ago: int) -> str:
    return (NOW - timedelta(days=days_ago)).isoformat()

def _profile(login: str, days_ago: int = 365) -> dict:
    return {"login": login, "created_at": _set_age(days_ago)}

def _make_issue(num: int, author: str, mod_id: str, review: str,
                labels: list[str] | None = None, created: str = "2026-06-01T00:00:00Z") -> dict:
    body = (
        f"### Mod Registry ID\n{mod_id}\n\n"
        f"### Registry Item ID\n{mod_id}\n\n"
        f"### Project Version Tested\n1.0.0\n\n"
        f"### Minecraft Version\n1.21\n\n"
        f"### Loader or platform\nfabric\n\n"
        f"### Your relationship to this project\nuser\n\n"
        f"### Review focus\nperformance\n\n"
        f"### Supporting evidence\nbenchmarks\n\n"
        f"### Known limitations or conflicts\nnone\n\n"
        f"### Technical Review\n{review}\n\n"
    )
    return {
        "number": num, "user": {"login": author}, "body": body,
        "labels": [{"name": l} for l in (labels or ["community-review"])],
        "created_at": created,
    }

def _reaction(rid: int, user: str, content: str, created: str = "2026-06-01T00:00:00Z") -> dict:
    return {"id": rid, "user": {"login": user}, "content": content, "created_at": created}

def _clear():
    gov.INJECTED_FETCH_ISSUES = None
    gov.INJECTED_FETCH_REACTIONS = None
    gov.INJECTED_USER_PROFILES = None

# ===================================================================
# resolve_governance_repo
# ===================================================================

class TestResolveGovernanceRepo(unittest.TestCase):
    def setUp(self):
        self._saved = {}
        for k in ("AGORA_GOVERNANCE_REPO", "AGORA_REGISTRY_REPO", "GITHUB_REPOSITORY"):
            self._saved[k] = os.environ.pop(k, None)
    def tearDown(self):
        for k, v in self._saved.items():
            if v is not None: os.environ[k] = v
            else: os.environ.pop(k, None)
    def test_cli_wins(self):
        self.assertEqual(gov.resolve_governance_repo("cli/r"), "cli/r")
    def test_env_var(self):
        os.environ["AGORA_GOVERNANCE_REPO"] = "gov/org"
        self.assertEqual(gov.resolve_governance_repo(None), "gov/org")
    def test_fallback_registry(self):
        os.environ["AGORA_REGISTRY_REPO"] = "reg/org"
        self.assertEqual(gov.resolve_governance_repo(None), "reg/org")
    def test_fallback_gh(self):
        os.environ["GITHUB_REPOSITORY"] = "gh/repo"
        self.assertEqual(gov.resolve_governance_repo(None), "gh/repo")
    def test_none_unset(self):
        self.assertIsNone(gov.resolve_governance_repo(None))

# ===================================================================
# Policy config
# ===================================================================

class TestPolicyConfig(unittest.TestCase):
    def test_production(self):
        c = gov.build_policy_config(gov.GovernancePolicy.PRODUCTION)
        self.assertEqual(c.account_age_days, 30); self.assertEqual(c.raid_threshold, 5); self.assertEqual(c.raid_window_minutes, 360)
    def test_sandbox(self):
        c = gov.build_policy_config(gov.GovernancePolicy.SANDBOX)
        self.assertEqual(c.account_age_days, 0); self.assertEqual(c.raid_threshold, 3); self.assertEqual(c.raid_window_minutes, 10)

# ===================================================================
# Event ID
# ===================================================================

class TestEventId(unittest.TestCase):
    def test_deterministic(self):
        a = gov.make_event_id("sodium", 42, [1,2,3])
        b = gov.make_event_id("sodium", 42, [1,2,3])
        self.assertEqual(a, b)
        self.assertTrue(a.startswith("sha256:"))
    def test_differs(self):
        self.assertNotEqual(gov.make_event_id("sodium", 42, [1,2]), gov.make_event_id("sodium", 42, [1,3]))

# ===================================================================
# parse_issue_form — exact heading labels
# ===================================================================

class TestParseIssueForm(unittest.TestCase):
    def _body(self) -> str:
        return (
            "### Mod Registry ID\nsodium\n\n"
            "### Registry Item ID\nsodium\n\n"
            "### Project Version Tested\n1.0.0\n\n"
            "### Minecraft Version\n1.21\n\n"
            "### Loader or platform\nfabric\n\n"
            "### Your relationship to this project\nuser\n\n"
            "### Review focus\nperformance\n\n"
            "### Supporting evidence\nbenchmarks\n\n"
            "### Known limitations or conflicts\nnone\n\n"
            "### Technical Review\n" + "x" * 100 + "\n"
        )
    def test_exact_field_keys(self):
        f = gov.parse_issue_form(self._body())
        self.assertEqual(f["mod_registry_id"], "sodium")
        self.assertEqual(f["registry_item_id"], "sodium")
        self.assertEqual(f["item_version"], "1.0.0")
        self.assertEqual(f["minecraft_version"], "1.21")
        self.assertEqual(f["loader"], "fabric")
        self.assertEqual(f["relationship"], "user")
        self.assertEqual(f["focus"], "performance")
        self.assertEqual(f["evidence"], "benchmarks")
        self.assertEqual(f["limitations"], "none")
        self.assertEqual(f["technical_review"], "x" * 100)
    def test_none_empty(self):
        self.assertEqual(gov.parse_issue_form(None), {})
        self.assertEqual(gov.parse_issue_form(""), {})
    def test_whitespace_normalized(self):
        f = gov.parse_issue_form("###   Mod   Registry   ID\nsodium\n")
        self.assertEqual(f.get("mod_registry_id"), "sodium")
    def test_crlf(self):
        f = gov.parse_issue_form("### Mod Registry ID\r\nsodium\r\n")
        self.assertEqual(f.get("mod_registry_id"), "sodium")
    def test_unknown_heading_preserved(self):
        f = gov.parse_issue_form("### Custom Heading\nval\n")
        self.assertEqual(f.get("custom heading"), "val")

# ===================================================================
# extract_reviews
# ===================================================================

class TestExtractReviews(unittest.TestCase):
    def test_valid(self):
        r = gov.extract_reviews([_make_issue(1, "alice", "sodium", "x" * 100)], {"sodium"})
        self.assertEqual(len(r["sodium"]), 1)
        self.assertEqual(r["sodium"][0]["author"], "alice")
        self.assertIn("item_version", r["sodium"][0])
        self.assertIn("minecraft_version", r["sodium"][0])
        self.assertIn("loader", r["sodium"][0])
        self.assertIn("relationship", r["sodium"][0])
        self.assertIn("focus", r["sodium"][0])
        self.assertIn("evidence", r["sodium"][0])
        self.assertIn("limitations", r["sodium"][0])
        self.assertIn("issue_url", r["sodium"][0])
    def test_skips_non_community_label(self):
        r = gov.extract_reviews([_make_issue(1, "alice", "sodium", "x" * 100, labels=["bug"])], {"sodium"})
        self.assertEqual(len(r.get("sodium", [])), 0)
    def test_skips_unknown_mod(self):
        r = gov.extract_reviews([_make_issue(1, "alice", "unknown", "x" * 100)], {"sodium"})
        self.assertEqual(len(r.get("unknown", [])), 0)
    def test_skips_short_review(self):
        r = gov.extract_reviews([_make_issue(1, "alice", "sodium", "short")], {"sodium"})
        self.assertEqual(len(r.get("sodium", [])), 0)
    def test_skips_no_author(self):
        r = gov.extract_reviews([{"number": 1, "body": "### Mod Registry ID\nsodium\n", "labels": [{"name": "community-review"}], "created_at": "2026-06-01T00:00:00Z"}], {"sodium"})
        self.assertEqual(len(r.get("sodium", [])), 0)
    def test_newest_per_author(self):
        r = gov.extract_reviews([
            _make_issue(1, "alice", "sodium", "x" * 100, created="2026-06-01T00:00:00Z"),
            _make_issue(2, "alice", "sodium", "y" * 100, created="2026-07-01T00:00:00Z"),
        ], {"sodium"})
        self.assertEqual(len(r["sodium"]), 1)
        self.assertEqual(r["sodium"][0]["technical_review"], "y" * 100)
    def test_multiple_authors(self):
        r = gov.extract_reviews([
            _make_issue(1, "alice", "sodium", "x" * 100),
            _make_issue(2, "bob", "sodium", "y" * 100),
        ], {"sodium"})
        self.assertEqual(len(r["sodium"]), 2)
    def test_any_issue_number(self):
        r = gov.extract_reviews([
            _make_issue(1, "alice", "sodium", "x" * 100),
            _make_issue(99, "bob", "sodium", "y" * 100),
        ], {"sodium"})
        self.assertEqual(len(r["sodium"]), 2)

# ===================================================================
# tally_item_votes
# ===================================================================

class TestTallyItemVotes(unittest.TestCase):
    def test_upvote(self):
        t = gov.tally_item_votes([_reaction(1,"alice","+1")], set())
        self.assertEqual(t["raw_up"], 1); self.assertEqual(t["raw_down"], 0)
    def test_downvote(self):
        t = gov.tally_item_votes([_reaction(2,"bob","-1")], set())
        self.assertEqual(t["raw_up"], 0); self.assertEqual(t["raw_down"], 1)
    def test_neutral_skipped(self):
        t = gov.tally_item_votes([_reaction(3,"c","heart")], set())
        self.assertEqual(t["raw_up"], 0); self.assertEqual(t["raw_down"], 0)
    def test_blacklist(self):
        t = gov.tally_item_votes([_reaction(4,"bob","+1")], {"bob"})
        self.assertEqual(t["raw_up"], 0)
    def test_dedup_same_direction(self):
        t = gov.tally_item_votes([_reaction(1,"a","+1"), _reaction(2,"a","+1")], set())
        self.assertEqual(t["raw_up"], 1)
    def test_conflict(self):
        t = gov.tally_item_votes([_reaction(1,"a","+1"), _reaction(2,"a","-1")], set())
        self.assertEqual(t["raw_up"], 0); self.assertEqual(t["raw_down"], 0)
        self.assertIn("a", t["conflict_users"])
    def test_all_reaction_ids(self):
        t = gov.tally_item_votes([_reaction(2,"a","+1"), _reaction(1,"b","-1")], set())
        self.assertEqual(t["all_reaction_ids"], [1,2])
    def test_lowercase(self):
        t = gov.tally_item_votes([_reaction(1,"AliceW","+1")], set())
        self.assertIn("alicew", t["non_conflict_pool"])

# ===================================================================
# Account eligibility
# ===================================================================

class TestEligibility(unittest.TestCase):
    def setUp(self):
        _clear()
    def test_production_old(self):
        gov.INJECTED_USER_PROFILES = {"a": _profile("a", 100)}
        self.assertTrue(gov.check_user_eligibility("a", gov.build_policy_config(gov.GovernancePolicy.PRODUCTION), token="t", cache={}, now=NOW))
    def test_production_young(self):
        gov.INJECTED_USER_PROFILES = {"b": _profile("b", 1)}
        self.assertFalse(gov.check_user_eligibility("b", gov.build_policy_config(gov.GovernancePolicy.PRODUCTION), token="t", cache={}, now=NOW))
    def test_sandbox_always(self):
        gov.INJECTED_USER_PROFILES = {"c": _profile("c", 0)}
        self.assertTrue(gov.check_user_eligibility("c", gov.build_policy_config(gov.GovernancePolicy.SANDBOX), token="t", cache={}, now=NOW))
    def test_boundary30(self):
        gov.INJECTED_USER_PROFILES = {"e": _profile("e", 30)}
        self.assertTrue(gov.check_user_eligibility("e", gov.build_policy_config(gov.GovernancePolicy.PRODUCTION), token="t", cache={}, now=NOW))
    def test_boundary29(self):
        gov.INJECTED_USER_PROFILES = {"f": _profile("f", 29)}
        self.assertFalse(gov.check_user_eligibility("f", gov.build_policy_config(gov.GovernancePolicy.PRODUCTION), token="t", cache={}, now=NOW))
    def test_missing_profile(self):
        gov.INJECTED_USER_PROFILES = {}
        self.assertFalse(gov.check_user_eligibility("x", gov.build_policy_config(gov.GovernancePolicy.PRODUCTION), token="t", cache={}, now=NOW))
    def test_cache(self):
        gov.INJECTED_USER_PROFILES = {"a": _profile("a", 100)}
        self.assertFalse(gov.check_user_eligibility("a", gov.build_policy_config(gov.GovernancePolicy.PRODUCTION), token="t", cache={"a": False}, now=NOW))

# ===================================================================
# Anomaly detection
# ===================================================================

class TestAnomaly(unittest.TestCase):
    def test_below_threshold(self):
        r = gov.detect_anomaly([1], [NOW - timedelta(minutes=5)], gov.PolicyConfig(raid_threshold=3, raid_window_minutes=10), now=NOW)
        self.assertFalse(r["is_anomaly"])
    def test_at_threshold(self):
        r = gov.detect_anomaly([1,2,3], [NOW - timedelta(minutes=1)]*3, gov.PolicyConfig(raid_threshold=3, raid_window_minutes=10), now=NOW)
        self.assertTrue(r["is_anomaly"]); self.assertEqual(r["affected_reaction_ids"], [1,2,3])
    def test_outside_window(self):
        r = gov.detect_anomaly([1,2,3], [NOW - timedelta(hours=1)]*3, gov.PolicyConfig(raid_threshold=3, raid_window_minutes=10), now=NOW)
        self.assertFalse(r["is_anomaly"])
    def test_partial_window(self):
        r = gov.detect_anomaly([1,2,3,4], [NOW - timedelta(minutes=1), NOW - timedelta(minutes=2), NOW - timedelta(hours=2), NOW - timedelta(hours=3)], gov.PolicyConfig(raid_threshold=2, raid_window_minutes=10), now=NOW)
        self.assertTrue(r["is_anomaly"]); self.assertEqual(r["affected_reaction_ids"], [1,2])
    def test_sandbox_anomaly(self):
        r = gov.detect_anomaly([1,2,3], [NOW - timedelta(seconds=30)]*3, gov.build_policy_config(gov.GovernancePolicy.SANDBOX), now=NOW)
        self.assertTrue(r["is_anomaly"])
    def test_production_anomaly(self):
        r = gov.detect_anomaly([1,2,3,4,5], [NOW - timedelta(minutes=30)]*5, gov.build_policy_config(gov.GovernancePolicy.PRODUCTION), now=NOW)
        self.assertTrue(r["is_anomaly"])

# ===================================================================
# State file I/O
# ===================================================================

class TestStateIO(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
    def tearDown(self):
        import shutil; shutil.rmtree(self.tmp, ignore_errors=True)
    def test_load_missing_vote_issues(self):
        self.assertEqual(gov.load_vote_issues(Path(self.tmp)/"v.json"), {"schema_version": 1, "items": {}})
    def test_load_missing_state(self):
        self.assertEqual(gov.load_governance_state(Path(self.tmp)/"s.json"), {"schema_version": 1, "events": []})
    def test_load_none_state(self):
        self.assertEqual(gov.load_governance_state(None), {"schema_version": 1, "events": []})
    def test_save_and_load(self):
        p = Path(self.tmp)/"s.json"
        d = {"schema_version": 1, "events": [{"event_id": "e1", "item_id": "sodium", "event_type": "vote_surge"}]}
        gov.save_governance_state(p, d)
        self.assertEqual(gov.load_governance_state(p), d)
    def test_malformed(self):
        p = Path(self.tmp)/"b.json"; p.write_text("{x", encoding="utf-8")
        self.assertEqual(gov.load_governance_state(p), {"schema_version": 1, "events": []})
    def test_atomic_write(self):
        p = Path(self.tmp)/"s.json"; gov.save_governance_state(p, {"events": []}); self.assertTrue(p.exists())
    def test_quarantine_not_written(self):
        p = Path(self.tmp)/"q.json"; p.write_text(json.dumps({"schema_version": 1, "decisions": []}), encoding="utf-8")
        orig = p.read_text(encoding="utf-8")
        _clear()
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: [_make_issue(1, "a", "sodium", "x"*100)]
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: [_reaction(1,"a","-1")]
        vi = Path(self.tmp)/"vi.json"; vi.write_text(json.dumps({"schema_version":1,"items":{"sodium":{"issue_number":1}}}), encoding="utf-8")
        gs = Path(self.tmp)/"gs.json"
        gov.run_governance_pipeline([{"id":"sodium","name":"Sodium"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.SANDBOX, governance_repo="owner/repo", token="t", blacklist=set(), vote_issues_path=vi, governance_state_in_path=gs, governance_state_out_path=gs, quarantine_decisions_path=p, discord_webhook_url=None)
        self.assertEqual(p.read_text(encoding="utf-8"), orig)

# ===================================================================
# Discord embed builder
# ===================================================================

class TestDiscordEmbed(unittest.TestCase):
    def test_none_in_off(self):
        self.assertIsNone(gov.build_discord_embed(mod_id="s", event_type="vote_surge", event_data={}, policy=gov.GovernancePolicy.PRODUCTION, mode=gov.GovernanceMode.OFF))
    def test_none_in_read_only(self):
        self.assertIsNone(gov.build_discord_embed(mod_id="s", event_type="vote_surge", event_data={}, policy=gov.GovernancePolicy.PRODUCTION, mode=gov.GovernanceMode.READ_ONLY))
    def test_quarantine_wording(self):
        r = gov.build_discord_embed(mod_id="sodium", event_type="vote_surge", event_data={"item_name":"Sodium","issue_url":"url","newly_quarantined":3,"total_quarantined":5,"before_score":"2","user_count":2,"event_id":"e1"}, policy=gov.GovernancePolicy.SANDBOX, mode=gov.GovernanceMode.MONITOR)
        self.assertIn("Vote Anomaly Detected", r["embeds"][0]["title"])
        names = [f["name"] for f in r["embeds"][0]["fields"]]
        for required in ("Item ID","Item Name","Issue URL","Newly Quarantined","Total Quarantined","Before Score","Users","Event ID","Decision"):
            self.assertIn(required, names)
    def test_new_review(self):
        r = gov.build_discord_embed(mod_id="s", event_type="new_review", event_data={"item_name":"S","issue_url":"u","review_count":1}, policy=gov.GovernancePolicy.PRODUCTION, mode=gov.GovernanceMode.MONITOR)
        self.assertIn("New Review", r["embeds"][0]["title"])
    def test_resolved(self):
        r = gov.build_discord_embed(mod_id="s", event_type="resolved", event_data={"item_name":"S","issue_url":"u","decision":"accepted"}, policy=gov.GovernancePolicy.PRODUCTION, mode=gov.GovernanceMode.MONITOR)
        self.assertIn("Resolved", r["embeds"][0]["title"])

# ===================================================================
# Pipeline mocked
# ===================================================================

class TestPipelineMocked(unittest.TestCase):
    def setUp(self):
        _clear(); self.tmp = tempfile.mkdtemp()
        self.vi = Path(self.tmp)/"vi.json"; self.gs = Path(self.tmp)/"gs.json"; self.qd = Path(self.tmp)/"qd.json"
        self.vi.write_text(json.dumps({"schema_version":1,"items":{"sodium":{"issue_number":1}}}), encoding="utf-8")
        self.gs.write_text(json.dumps({"schema_version":1,"events":[]}), encoding="utf-8")
        self.qd.write_text(json.dumps({"schema_version":1,"decisions":[]}), encoding="utf-8")
        gov.INJECTED_USER_PROFILES = {"alice": _profile("alice", 100), "bob": _profile("bob", 100)}
    def tearDown(self):
        _clear(); import shutil; shutil.rmtree(self.tmp, ignore_errors=True)

    def test_off_mode(self):
        r = gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.OFF, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertEqual(r, {})

    def test_no_token(self):
        r = gov.run_governance_pipeline([{"id":"sodium"}], mode=gov.GovernanceMode.READ_ONLY, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token=None, blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertEqual(r, {})

    def test_no_repo(self):
        r = gov.run_governance_pipeline([{"id":"sodium"}], mode=gov.GovernanceMode.READ_ONLY, policy=gov.GovernancePolicy.PRODUCTION, governance_repo=None, token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertEqual(r, {})

    def test_no_mapped_items(self):
        self.vi.write_text(json.dumps({"schema_version":1,"items":{}}), encoding="utf-8")
        r = gov.run_governance_pipeline([{"id":"sodium"}], mode=gov.GovernanceMode.READ_ONLY, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertEqual(r, {})

    def test_full_pipeline(self):
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: [_make_issue(1,"alice","sodium","x"*100)]
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: [_reaction(1,"alice","+1"), _reaction(2,"bob","-1")]
        r = gov.run_governance_pipeline([{"id":"sodium","name":"Sodium"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="owner/repo", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertIn("sodium", r)
        self.assertEqual(r["sodium"]["raw_upvotes"], 1)
        self.assertEqual(r["sodium"]["raw_downvotes"], 1)
        self.assertEqual(r["sodium"]["counted_upvotes"], 1)
        self.assertEqual(r["sodium"]["counted_downvotes"], 1)
        self.assertEqual(r["sodium"]["quarantined_upvotes"], 0)
        self.assertEqual(r["sodium"]["quarantined_downvotes"], 0)

    def test_state_persistence_no_new_event_if_unchanged(self):
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: [_make_issue(1,"alice","sodium","x"*100)]
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: [_reaction(1,"alice","+1")]
        r1 = gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertIn("sodium", r1)
        r2 = gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertIn("sodium", r2)

    def test_accepted_decision_not_excluded(self):
        existing_event_id = gov.make_event_id("sodium", 1, list(range(1, 6)))
        self.gs.write_text(json.dumps({"schema_version": 1, "events": [{
            "event_id": existing_event_id, "item_id": "sodium", "event_type": "vote_surge",
            "status": "pending", "detected_at": "2026-07-27T00:00:00Z",
            "affected_reactions": [1,2,3,4,5],
            "details_json": "{}",
        }]}), encoding="utf-8")
        self.qd.write_text(json.dumps({"schema_version": 1, "decisions": [{"event_id": existing_event_id, "status": "accepted"}]}), encoding="utf-8")
        gov.INJECTED_USER_PROFILES = {
            "alice": _profile("alice", 100), "bob": _profile("bob", 100),
            "charlie": _profile("charlie", 100), "dave": _profile("dave", 100),
            "eve": _profile("eve", 100), "frank": _profile("frank", 100),
        }
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: [_make_issue(1,"alice","sodium","x"*100)]
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: [
            _reaction(1,"alice","+1"), _reaction(2,"bob","-1"),
            _reaction(3,"charlie","-1"), _reaction(4,"dave","-1"),
            _reaction(5,"eve","-1"), _reaction(6,"frank","-1"),
        ]

        # After accepted decision, anomaly should still trigger but reaction IDs
        # from accepted event should NOT be excluded from counted
        r = gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        # alice +1 → counted = 1 upvote
        # bob..frank -1 → 5 downvotes, but accepted event restored them so they count
        self.assertIn("sodium", r)

    def test_pending_event_excludes_reactions(self):
        existing_event_id = gov.make_event_id("sodium", 1, [1,2,3,4,5])
        self.gs.write_text(json.dumps({"schema_version": 1, "events": [{
            "event_id": existing_event_id, "item_id": "sodium", "event_type": "vote_surge",
            "status": "pending", "detected_at": "2026-07-27T00:00:00Z",
            "affected_reactions": [1,2,3,4,5],
            "details_json": "{}",
        }]}), encoding="utf-8")
        gov.INJECTED_USER_PROFILES = {
            "alice": _profile("alice", 100), "bob": _profile("bob", 100),
            "charlie": _profile("charlie", 100), "dave": _profile("dave", 100),
            "eve": _profile("eve", 100),
        }
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: [_make_issue(1,"alice","sodium","x"*100)]
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: [
            _reaction(1,"alice","+1"), _reaction(2,"bob","-1"),
            _reaction(3,"charlie","-1"), _reaction(4,"dave","-1"),
            _reaction(5,"eve","-1"),
        ]
        r = gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        # alice reaction_id=1 is quarantined (in pending event) → counted_up=0
        # bob..eve reaction_ids 2-5 quarantined → counted_down=0
        self.assertEqual(r["sodium"]["counted_upvotes"], 0)
        self.assertEqual(r["sodium"]["counted_downvotes"], 0)
        self.assertEqual(r["sodium"]["quarantined_upvotes"], 1)
        self.assertEqual(r["sodium"]["quarantined_downvotes"], 4)

    def test_state_written_only_in_monitor(self):
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: [_make_issue(1,"alice","sodium","x"*100)]
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: [_reaction(1,"alice","+1")]
        gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.READ_ONLY, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        state = gov.load_governance_state(self.gs)
        self.assertEqual(len(state.get("events", [])), 0)

# ===================================================================
# DB enrichment (exact columns)
# ===================================================================

class TestDBEnrichment(unittest.TestCase):
    def setUp(self):
        self.conn = __import__("sqlite3").connect(":memory:")
        self.conn.execute("""
            CREATE TABLE IF NOT EXISTS governance_summary (
                item_id TEXT NOT NULL PRIMARY KEY,
                vote_issue_number INTEGER,
                vote_issue_url TEXT,
                raw_upvotes INTEGER NOT NULL DEFAULT 0,
                raw_downvotes INTEGER DEFAULT 0,
                counted_upvotes INTEGER DEFAULT 0,
                counted_downvotes INTEGER DEFAULT 0,
                quarantined_upvotes INTEGER DEFAULT 0,
                quarantined_downvotes INTEGER DEFAULT 0,
                conflicted_users TEXT,
                status_reason TEXT,
                compiled_at TEXT NOT NULL
            )
        """)
        self.conn.execute("""
            CREATE TABLE IF NOT EXISTS governance_events (
                event_id TEXT NOT NULL PRIMARY KEY,
                item_id TEXT NOT NULL,
                event_type TEXT,
                status TEXT,
                detected_at TEXT,
                affected_reactions TEXT,
                details_json TEXT
            )
        """)
    def tearDown(self):
        self.conn.close()

    def test_enrich_summary_columns(self):
        r = {"sodium": {"vote_issue_number": 1, "vote_issue_url": "url", "raw_upvotes": 5, "raw_downvotes": 3, "counted_upvotes": 4, "counted_downvotes": 2, "quarantined_upvotes": 1, "quarantined_downvotes": 1, "conflicted_users": [], "status_reason": "normal"}}
        gov.enrich_governance_summary(self.conn, r)
        row = self.conn.execute("SELECT item_id, raw_upvotes, counted_upvotes, quarantined_upvotes FROM governance_summary").fetchone()
        self.assertEqual(row[0], "sodium"); self.assertEqual(row[1], 5); self.assertEqual(row[2], 4); self.assertEqual(row[3], 1)

    def test_enrich_summary_noop_empty(self):
        gov.enrich_governance_summary(self.conn, {})
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM governance_summary").fetchone()[0], 0)

    def test_enrich_events_exact_columns(self):
        r = {"sodium": {"event_id": "sha256:abc", "vote_issue_number": 1, "raw_upvotes": 3, "counted_upvotes": 2}, "_meta": {}}
        gov.enrich_governance_events(self.conn, r)
        row = self.conn.execute("SELECT event_id, item_id, event_type, status FROM governance_events").fetchone()
        self.assertEqual(row[0], "sha256:abc"); self.assertEqual(row[1], "sodium"); self.assertEqual(row[2], "vote_surge")

    def test_enrich_events_skips_no_event_id(self):
        r = {"sodium": {"event_id": None}, "_meta": {}}
        gov.enrich_governance_events(self.conn, r)
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM governance_events").fetchone()[0], 0)

    def test_enrich_top_reviews_exact_fields(self):
        item = {"id": "sodium"}
        results = {"sodium": {
            "reviews": [{
                "author": "alice", "technical_review": "x" * 100,
                "item_version": "1.0.0", "minecraft_version": "1.21",
                "loader": "fabric", "relationship": "user", "focus": "performance",
                "evidence": "benchmarks", "limitations": "none",
                "issue_url": "https://github.com/issues/1",
                "issue_number": 1, "created_at": "2026-06-01T00:00:00Z",
            }],
            "counted_upvotes": 3, "counted_downvotes": 1, "raw_upvotes": 5,
            "raw_downvotes": 3, "quarantined_upvotes": 1, "quarantined_downvotes": 1,
            "status_reason": "normal", "vote_issue_url": "url",
            "review_count": 1,
        }}
        gov.enrich_registry_item_scores(item, results)
        self.assertEqual(item["_governance_counted_upvotes"], 3)
        self.assertEqual(item["_governance_counted_downvotes"], 1)
        self.assertEqual(item["_governance_net_score"], 2)
        self.assertEqual(item["_governance_raw_upvotes"], 5)
        self.assertEqual(item["_governance_raw_downvotes"], 3)
        self.assertEqual(item["_governance_quarantined_upvotes"], 1)
        self.assertEqual(item["_governance_quarantined_downvotes"], 1)
        self.assertIn("_governance_top_reviews", item)
        tr = item["_governance_top_reviews"][0]
        self.assertEqual(tr["item_version"], "1.0.0")
        self.assertEqual(tr["minecraft_version"], "1.21")
        self.assertEqual(tr["loader"], "fabric")
        self.assertEqual(tr["relationship"], "user")
        self.assertEqual(tr["focus"], "performance")
        self.assertEqual(tr["evidence"], "benchmarks")
        self.assertEqual(tr["limitations"], "none")
        self.assertEqual(tr["issue_url"], "https://github.com/issues/1")

    def test_enrich_item_scores_no_data(self):
        item = {"id": "sodium"}
        gov.enrich_registry_item_scores(item, {})
        self.assertNotIn("_governance_counted_upvotes", item)

# ===================================================================
# Enums
# ===================================================================

class TestEnums(unittest.TestCase):
    def test_policy(self):
        self.assertEqual(gov.GovernancePolicy.PRODUCTION.value, "production")
        self.assertEqual(gov.GovernancePolicy.SANDBOX.value, "sandbox")
    def test_mode(self):
        self.assertEqual(gov.GovernanceMode.OFF.value, "off")
        self.assertEqual(gov.GovernanceMode.READ_ONLY.value, "read-only")
        self.assertEqual(gov.GovernanceMode.MONITOR.value, "monitor")
    def test_decision(self):
        self.assertEqual(gov.DecisionStatus.PENDING.value, "pending")
        self.assertEqual(gov.DecisionStatus.REJECTED.value, "rejected")
        self.assertEqual(gov.DecisionStatus.ACCEPTED.value, "accepted")

if __name__ == "__main__":
    unittest.main()
