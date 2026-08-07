#!/usr/bin/env python3
from __future__ import annotations

"""Integration tests for governance with compile.py schema7 DDL. No real network."""

import json
import os
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__)))
import compile as _compile
import governance as gov

# ===================================================================
# Schema 7 DDL exact columns
# ===================================================================

class TestSchema7DDL(unittest.TestCase):
    def setUp(self):
        self.conn = sqlite3.connect(":memory:")
        self.conn.execute("PRAGMA foreign_keys = ON")
        _compile.create_tables(self.conn)
        self.conn.execute(
            "INSERT INTO registry_items (id, name, content_type, download_strategy, source_identifier, sha256) "
            "VALUES ('sodium', 'Sodium', 'mod', 'modrinth_id', 'sodium', '" + "a" * 64 + "')",
        )

    def tearDown(self):
        self.conn.close()

    def test_governance_summary_exact_columns(self):
        cols = {row[1]: row[2] for row in
                self.conn.execute("PRAGMA table_info(governance_summary)").fetchall()}
        expected = {
            "item_id": "TEXT", "vote_issue_number": "INTEGER",
            "vote_issue_url": "TEXT", "raw_upvotes": "INTEGER",
            "raw_downvotes": "INTEGER", "counted_upvotes": "INTEGER",
            "counted_downvotes": "INTEGER", "quarantined_upvotes": "INTEGER",
            "quarantined_downvotes": "INTEGER", "conflicted_users": "INTEGER",
            "status_reason": "TEXT", "compiled_at": "TEXT",
        }
        for col, typ in expected.items():
            self.assertIn(col, cols, f"Missing column: {col}")
            self.assertEqual(cols[col].upper(), typ, f"Column {col} type mismatch")

    def test_governance_events_exact_columns(self):
        cols = {row[1]: row[2] for row in
                self.conn.execute("PRAGMA table_info(governance_events)").fetchall()}
        expected = {
            "event_id": "TEXT", "item_id": "TEXT", "event_type": "TEXT",
            "status": "TEXT", "detected_at": "TEXT", "affected_reactions": "INTEGER",
            "details_json": "TEXT",
        }
        for col, typ in expected.items():
            self.assertIn(col, cols, f"Missing column: {col}")
            self.assertEqual(cols[col].upper(), typ, f"Column {col} type mismatch")

    def test_governance_summary_no_old_columns(self):
        cols = {row[1] for row in
                self.conn.execute("PRAGMA table_info(governance_summary)").fetchall()}
        for old in ("mod_id", "status", "governance_mode", "governance_policy",
                    "review_count", "unique_reviewers", "vote_tally_json", "generated_at",
                    "quarantine_decision_id"):
            self.assertNotIn(old, cols, f"Old column {old} must not exist in governance_summary")

    def test_governance_events_no_old_columns(self):
        cols = {row[1] for row in
                self.conn.execute("PRAGMA table_info(governance_events)").fetchall()}
        for old in ("id", "mod_id", "event_data_json", "created_at"):
            self.assertNotIn(old, cols, f"Old column {old} must not exist in governance_events")

    def test_governance_summary_insert_and_select(self):
        self.conn.execute(
            "INSERT INTO governance_summary (item_id, vote_issue_number, vote_issue_url, "
            "raw_upvotes, raw_downvotes, counted_upvotes, counted_downvotes, "
            "quarantined_upvotes, quarantined_downvotes, conflicted_users, status_reason, compiled_at) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            ("sodium", 1, "https://github.com/o/r/issues/1",
              10, 5, 8, 4, 2, 1, 1, "normal", "2026-07-27T00:00:00Z"),
        )
        row = self.conn.execute(
            "SELECT item_id, raw_upvotes, counted_upvotes, quarantined_upvotes "
            "FROM governance_summary WHERE item_id = ?", ("sodium",),
        ).fetchone()
        self.assertEqual(row[0], "sodium")
        self.assertEqual(row[1], 10)
        self.assertEqual(row[2], 8)
        self.assertEqual(row[3], 2)

    def test_governance_events_insert_and_select(self):
        self.conn.execute(
            "INSERT INTO governance_events (event_id, item_id, event_type, status, "
            "detected_at, affected_reactions, details_json) "
            "VALUES (?, ?, ?, ?, ?, ?, ?)",
            ("sha256:abc123", "sodium", "vote_surge", "pending",
             "2026-07-27T00:00:00Z", "[1,2,3]", '{"raw_up":5,"raw_down":3}'),
        )
        row = self.conn.execute(
            "SELECT event_id, item_id, event_type, status FROM governance_events"
        ).fetchone()
        self.assertEqual(row[0], "sha256:abc123")
        self.assertEqual(row[1], "sodium")
        self.assertEqual(row[2], "vote_surge")
        self.assertEqual(row[3], "pending")


# ===================================================================
# Governance enrichment integration
# ===================================================================

class TestGovernanceEnrichment(unittest.TestCase):
    def setUp(self):
        self.conn = sqlite3.connect(":memory:")
        self.conn.execute("PRAGMA foreign_keys = ON")
        _compile.create_tables(self.conn)
        self.conn.execute(
            "INSERT INTO registry_items (id, name, content_type, download_strategy, source_identifier, sha256) "
            "VALUES ('sodium', 'Sodium', 'mod', 'modrinth_id', 'sodium', '" + "a" * 64 + "')",
        )
        self.conn.execute(
            "INSERT INTO registry_items (id, name, content_type, download_strategy, source_identifier, sha256) "
            "VALUES ('iris', 'Iris', 'mod', 'modrinth_id', 'iris', '" + "b" * 64 + "')",
        )

    def tearDown(self):
        self.conn.close()

    def test_enrich_summary(self):
        results = {
            "sodium": {"vote_issue_number": 1, "vote_issue_url": "url",
                       "raw_upvotes": 5, "raw_downvotes": 3,
                       "counted_upvotes": 4, "counted_downvotes": 2,
                       "quarantined_upvotes": 1, "quarantined_downvotes": 1,
                       "conflicted_users": [], "status_reason": "normal"},
            "_meta": {},
        }
        gov.enrich_governance_summary(self.conn, results)
        row = self.conn.execute(
            "SELECT item_id, raw_upvotes, counted_upvotes, quarantined_upvotes "
            "FROM governance_summary"
        ).fetchone()
        self.assertEqual(row[0], "sodium")
        self.assertEqual(row[1], 5)
        self.assertEqual(row[2], 4)
        self.assertEqual(row[3], 1)

    def test_enrich_events(self):
        event = {"event_id": "sha256:xyz", "item_id": "sodium", "event_type": "vote_surge", "status": "pending", "detected_at": "2026-07-27T00:00:00Z", "affected_reactions": [1, 2], "details_json": "{}"}
        results = {
            "sodium": {"event_id": "sha256:xyz", "vote_issue_number": 1,
                       "raw_upvotes": 10, "counted_upvotes": 8,
                       "quarantined_upvotes": 2, "conflicted_users": []},
            "_meta": {"events": [event]},
        }
        gov.enrich_governance_events(self.conn, results)
        row = self.conn.execute(
            "SELECT event_id, item_id, event_type, status FROM governance_events"
        ).fetchone()
        self.assertEqual(row[0], "sha256:xyz")
        self.assertEqual(row[1], "sodium")
        self.assertEqual(row[2], "vote_surge")
        self.assertEqual(row[3], "pending")

    def test_item_scores(self):
        results = {
            "sodium": {
                "counted_upvotes": 5, "counted_downvotes": 2,
                "raw_upvotes": 8, "raw_downvotes": 4,
                "quarantined_upvotes": 1, "quarantined_downvotes": 1,
                "status_reason": "normal", "registry_status": "active",
                "vote_issue_url": "url", "velocity": 1.5,
                "review_count": 2,
                "reviews": [{"author": "alice", "technical_review": "x"*100,
                             "item_version": "1.0.0", "minecraft_version": "1.21",
                             "loader": "fabric", "relationship": "user",
                             "focus": "performance", "evidence": "bench",
                             "limitations": "none",
                             "issue_url": "https://github.com/issues/1",
                             "issue_number": 1, "created_at": "2026-06-01T00:00:00Z"}],
            },
        }
        item = {"id": "sodium"}
        gov.enrich_registry_item_scores(item, results)
        self.assertEqual(item["_governance_counted_upvotes"], 5)
        self.assertEqual(item["_governance_counted_downvotes"], 2)
        self.assertEqual(item["_governance_net_score"], 3)
        self.assertEqual(item["_governance_raw_upvotes"], 8)
        self.assertEqual(item["_governance_raw_downvotes"], 4)
        self.assertEqual(item["_governance_quarantined_upvotes"], 1)
        self.assertEqual(item["_governance_quarantined_downvotes"], 1)
        self.assertEqual(item["_governance_registry_status"], "active")
        self.assertEqual(item["_governance_velocity"], 1.5)
        self.assertEqual(len(item["_governance_top_reviews"]), 1)
        tr = item["_governance_top_reviews"][0]
        self.assertEqual(tr["item_version"], "1.0.0")
        self.assertEqual(tr["loader"], "fabric")
        self.assertEqual(tr["evidence"], "bench")

    def test_enrich_tables(self):
        event = {"event_id": "sha256:e1", "item_id": "sodium", "event_type": "vote_surge", "status": "pending", "detected_at": "2026-07-27T00:00:00Z", "affected_reactions": [1, 2, 3, 4], "details_json": "{}"}
        results = {
            "sodium": {"event_id": "sha256:e1", "vote_issue_number": 1, "vote_issue_url": "u",
                       "raw_upvotes": 3, "raw_downvotes": 1, "counted_upvotes": 2,
                       "counted_downvotes": 1, "quarantined_upvotes": 1,
                       "quarantined_downvotes": 0, "conflicted_users": [],
                       "status_reason": "vote_surge", "affected_reaction_ids": [1,2,3,4]},
            "iris": {"event_id": None, "vote_issue_number": 2, "vote_issue_url": "u2",
                     "raw_upvotes": 1, "raw_downvotes": 0, "counted_upvotes": 1,
                     "counted_downvotes": 0, "quarantined_upvotes": 0,
                     "quarantined_downvotes": 0, "conflicted_users": [],
                     "status_reason": "normal"},
            "_meta": {"events": [event]},
        }
        gov.enrich_governance_tables(self.conn, results)
        summary_rows = self.conn.execute("SELECT item_id, raw_upvotes FROM governance_summary ORDER BY item_id").fetchall()
        self.assertEqual(len(summary_rows), 2)
        self.assertEqual(summary_rows[0][0], "iris")
        self.assertEqual(summary_rows[1][0], "sodium")
        event_rows = self.conn.execute("SELECT item_id, event_type FROM governance_events").fetchall()
        self.assertEqual(len(event_rows), 1)
        self.assertEqual(event_rows[0][0], "sodium")


# ===================================================================
# End-to-end with mocked pipeline
# ===================================================================

class TestEndToEndMocked(unittest.TestCase):
    def setUp(self):
        gov.INJECTED_USER_PROFILES = {}
        self.tmp = tempfile.mkdtemp()
        self.vi = Path(self.tmp)/"vi.json"
        self.gs = Path(self.tmp)/"gs.json"
        self.qd = Path(self.tmp)/"qd.json"
        self.vi.write_text(json.dumps({"schema_version":1,"items":{}}), encoding="utf-8")
        self.gs.write_text(json.dumps({"schema_version":1,"events":[]}), encoding="utf-8")
        self.qd.write_text(json.dumps({"schema_version":1,"decisions":[]}), encoding="utf-8")

    def tearDown(self):
        gov.INJECTED_FETCH_ISSUES = None
        gov.INJECTED_FETCH_REACTIONS = None
        gov.INJECTED_USER_PROFILES = None
        import shutil; shutil.rmtree(self.tmp, ignore_errors=True)

    def test_off_mode(self):
        r = gov.run_governance_pipeline([{"id":"s","name":"S"}], mode=gov.GovernanceMode.OFF, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertEqual(r, {})

    def test_full_monitor(self):
        self.vi.write_text(json.dumps({"schema_version":1,"items":{"sodium":{"issue_number":1}}}), encoding="utf-8")
        gov.INJECTED_USER_PROFILES = {"alice": {"login":"alice","created_at":"2025-01-01T00:00:00Z"}}
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: [
            {"number":1,"user":{"login":"alice"},
             "body":"### Mod Registry ID\nsodium\n\n### Registry Item ID\nsodium\n\n### Technical Review\n"+("x"*100)+"\n",
             "labels":[{"name":"registry-vote"}],"created_at":"2026-06-01T00:00:00Z"},
        ]
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: [
            {"id":1,"user":{"login":"alice"},"content":"+1","created_at":"2026-06-01T00:00:00Z"},
            {"id":2,"user":{"login":"bob"},"content":"-1","created_at":"2026-06-01T00:00:00Z"},
        ]
        r = gov.run_governance_pipeline([{"id":"sodium","name":"Sodium"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="owner/repo", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertIn("sodium", r)
        self.assertEqual(r["sodium"]["raw_upvotes"], 1)
        self.assertEqual(r["sodium"]["raw_downvotes"], 1)
        self.assertIn("_meta", r)
        state = gov.load_governance_state(self.gs)
        self.assertEqual(len(state["events"]), 0)  # no anomaly, no events
        # State includes governance_repository and policy
        self.assertIn("governance_repository", state)
        self.assertIn("policy", state)

    def test_quarantine_untouched(self):
        self.vi.write_text(json.dumps({"schema_version":1,"items":{"sodium":{"issue_number":1}}}), encoding="utf-8")
        orig = self.qd.read_text(encoding="utf-8")
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: []
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: []
        r = gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertEqual(self.qd.read_text(encoding="utf-8"), orig)

    def test_unknown_item_in_vi_still_emits_reviews(self):
        # Even with an unknown item in vote_issues, items with reviews still appear
        self.vi.write_text(json.dumps({"schema_version":1,"items":{"unknown":{"issue_number":1}}}), encoding="utf-8")
        _body = (
            "### Mod Registry ID\nsodium\n\n"
            "### Registry Item ID\nsodium\n\n"
            "### Project Version Tested\n1.0.0\n\n"
            "### Minecraft Version\n1.21\n\n"
            "### Loader or platform\nfabric\n\n"
            "### Your relationship to this project\nuser\n\n"
            "### Review focus\nperformance\n\n"
            "### Technical Review\n" + ("x"*100) + "\n"
        )
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: [
            {"number":1,"user":{"login":"alice"}, "body":_body,
             "labels":[{"name":"community-review"}],"created_at":"2026-06-01T00:00:00Z"},
        ]
        gov.INJECTED_USER_PROFILES = {"alice": {"login":"alice","created_at":"2025-01-01T00:00:00Z"}}
        r = gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.READ_ONLY, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        # sodium is a known registry item so it should appear with reviews
        self.assertIn("sodium", r)
        self.assertEqual(r["sodium"]["review_count"], 1)
        # unknown was in vote_issues but not in items list — it won't appear
        self.assertNotIn("unknown", r)

    def test_existing_accepted_state_persists(self):
        self.vi.write_text(json.dumps({"schema_version":1,"items":{"sodium":{"issue_number":1}}}), encoding="utf-8")
        from datetime import timezone as _tz
        detected_bucket = "2026-07-27T00:00:00+00:00"
        eid = gov.make_stable_event_id("sodium", 1, detected_bucket)
        self.gs.write_text(json.dumps({"schema_version":1,"governance_repository":"o/r","policy":"production","events":[{
            "event_id": eid, "item_id": "sodium", "event_type": "vote_surge",
            "status": "accepted", "detected_at": "2026-07-27T00:00:00Z",
            "affected_reactions": [1,2,3,4,5], "details_json": "{}",
        }]}), encoding="utf-8")
        self.qd.write_text(json.dumps({"schema_version":1,"decisions":[{"event_id":eid,"status":"accepted"}]}), encoding="utf-8")
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: []
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: [_reaction(1,"a","+1")]
        gov.INJECTED_USER_PROFILES = {"a": {"login":"a","created_at":"2025-01-01T00:00:00Z"}}
        r = gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertIn("sodium", r)
        self.assertEqual(r["sodium"]["registry_status"], "active")

    def test_pending_event_keeps_under_review_after_window(self):
        # Pending event persists and keeps item under_review even when no anomaly
        self.vi.write_text(json.dumps({"schema_version":1,"items":{"sodium":{"issue_number":1}}}), encoding="utf-8")
        from datetime import timezone as _tz
        detected_bucket = "2026-07-20T00:00:00+00:00"
        eid = gov.make_stable_event_id("sodium", 1, detected_bucket)
        self.gs.write_text(json.dumps({"schema_version":1,"governance_repository":"o/r","policy":"production","events":[{
            "event_id": eid, "item_id": "sodium", "event_type": "vote_surge",
            "status": "pending", "detected_at": "2026-07-20T00:00:00Z",
            "affected_reactions": [1,2,3,4,5], "details_json": "{}",
        }]}), encoding="utf-8")
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: [
            {"number":1,"user":{"login":"alice"},"body":"### Mod Registry ID\nsodium\n\n",
             "labels":[{"name":"registry-vote"}],"created_at":"2026-06-01T00:00:00Z"},
        ]
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: [_reaction(1,"a","+1")]
        gov.INJECTED_USER_PROFILES = {"a": {"login":"a","created_at":"2025-01-01T00:00:00Z"}}
        r = gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        # Pending event persists; reaction 1 still quarantined; item stays under_review
        self.assertIn("sodium", r)
        self.assertEqual(r["sodium"]["quarantined_upvotes"], 1)
        self.assertEqual(r["sodium"]["counted_upvotes"], 0)
        self.assertEqual(r["sodium"]["registry_status"], "under_review")
        self.assertEqual(r["sodium"]["status_reason"], "pending_vote_quarantine")

    def test_rejected_decision_excludes_votes_but_resolves_under_review(self):
        self.vi.write_text(json.dumps({"schema_version":1,"items":{"sodium":{"issue_number":1}}}), encoding="utf-8")
        eid = gov.make_stable_event_id("sodium", 1, "2026-07-20T00:00:00+00:00")
        self.gs.write_text(json.dumps({"schema_version":1,"governance_repository":"o/r","policy":"production","events":[{
            "event_id": eid, "item_id": "sodium", "event_type": "vote_surge",
            "status": "pending", "detected_at": "2026-07-20T00:00:00Z",
            "affected_reactions": [1], "details_json": json.dumps({"issue_number": 1}),
        }]}), encoding="utf-8")
        self.qd.write_text(json.dumps({"schema_version":1,"decisions":[{"event_id":eid,"status":"rejected"}]}), encoding="utf-8")
        gov.INJECTED_FETCH_ISSUES = lambda o,r,token: [
            {"number":1,"user":{"login":"alice"},"body":"### Mod Registry ID\nsodium\n\n",
             "labels":[{"name":"registry-vote"}],"created_at":"2026-06-01T00:00:00Z"},
        ]
        gov.INJECTED_FETCH_REACTIONS = lambda o,r,i,token: [_reaction(1,"a","-1")]
        gov.INJECTED_USER_PROFILES = {"a": {"login":"a","created_at":"2025-01-01T00:00:00Z"}}
        r = gov.run_governance_pipeline([{"id":"sodium","name":"S"}], mode=gov.GovernanceMode.MONITOR, policy=gov.GovernancePolicy.PRODUCTION, governance_repo="o/r", token="t", blacklist=set(), vote_issues_path=self.vi, governance_state_in_path=self.gs, governance_state_out_path=self.gs, quarantine_decisions_path=self.qd, discord_webhook_url=None)
        self.assertEqual(r["sodium"]["quarantined_downvotes"], 1)
        self.assertEqual(r["sodium"]["counted_downvotes"], 0)
        self.assertEqual(r["sodium"]["registry_status"], "active")
        self.assertEqual(r["sodium"]["status_reason"], "normal")
        saved = json.loads(self.gs.read_text(encoding="utf-8"))
        self.assertEqual(saved["events"][0]["status"], "rejected")


def _reaction(rid, user, content, created="2026-07-27T00:00:00Z"):
    return {"id": rid, "user": {"login": user}, "content": content, "created_at": created}

if __name__ == "__main__":
    unittest.main()
