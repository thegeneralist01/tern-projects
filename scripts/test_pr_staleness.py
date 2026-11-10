import json
import unittest
from datetime import datetime, timedelta, timezone

from pr_staleness import BOT, MARKER, activity_time, fingerprint, stale_label


class StalenessTests(unittest.TestCase):
    def setUp(self):
        self.start = datetime(2026, 1, 1, tzinfo=timezone.utc)
        self.issue = {
            "created_at": self.date(0), "updated_at": self.date(0),
            "title": "Fix search", "body": "A useful change",
        }
        self.pull = {"head": {"sha": "original"}}
        self.marker = {
            "id": 123, "updated_at": self.date(7),
            "body": MARKER + json.dumps({
                "last_activity": self.date(0),
                "fingerprint": fingerprint(self.issue, self.pull),
            }) + "\n-->",
        }

    def date(self, days):
        return (self.start + timedelta(days=days)).isoformat()

    def activity(self, events=(), comments=(), review_comments=(), day=21):
        return activity_time(self.issue, self.pull, list(comments), list(events),
                             list(review_comments), self.marker, self.start + timedelta(days=day))

    def test_exact_week_boundaries(self):
        for days, expected in ((0, None), (7, "soft-stale"), (14, "medium-stale"), (21, "hard-stale")):
            with self.subTest(days=days):
                now = self.start + timedelta(days=days)
                self.assertEqual(stale_label(self.start, now), expected)
                if days:
                    previous = {7: None, 14: "soft-stale", 21: "medium-stale"}[days]
                    self.assertEqual(stale_label(self.start, now - timedelta(seconds=1)), previous)

    def test_own_comment_and_label_changes_do_not_restart_clock(self):
        events = [{"event": "commented", "id": 123, "created_at": self.date(7),
                   "updated_at": self.date(7)}]
        for day, label in ((7, "soft-stale"), (14, "medium-stale"), (21, "hard-stale")):
            events.append({"event": "labeled", "created_at": self.date(day),
                           "actor": {"login": BOT}, "label": {"name": label}})
            if day > 7:
                events.append({"event": "unlabeled", "created_at": self.date(day),
                               "actor": {"login": BOT}, "label": {"name": "soft-stale"}})
            self.issue["updated_at"] = self.date(day)
            with self.subTest(day=day):
                last = self.activity(events, [self.marker], day=day)
                self.assertEqual(last, self.start)
                self.assertEqual(stale_label(last, self.start + timedelta(days=day)), label)

    def test_comment_or_review_revives_stale_pr(self):
        for source in ("comment", "review", "inline"):
            with self.subTest(source=source):
                comment = {"id": 456, "updated_at": self.date(20)}
                events = [{"event": "reviewed", "submitted_at": self.date(20)}] if source == "review" else []
                last = self.activity(events,
                                     [comment] if source == "comment" else [],
                                     [comment] if source == "inline" else [])
                self.assertEqual(last, self.start + timedelta(days=20))
                self.assertIsNone(stale_label(last, self.start + timedelta(days=21)))
                self.assertEqual(stale_label(last, self.start + timedelta(days=27)), "soft-stale")

    def test_push_or_edit_hidden_by_our_timestamp_is_not_lost(self):
        self.issue["updated_at"] = self.date(7)
        for change in ("push", "title", "body"):
            with self.subTest(change=change):
                if change == "push":
                    self.pull["head"]["sha"] = "pushed-old-commit"
                else:
                    self.issue[change] = "Edited"
                self.assertEqual(self.activity(day=8), self.start + timedelta(days=8))
                self.pull["head"]["sha"] = "original"
                self.issue.update(title="Fix search", body="A useful change")

    def test_regular_activity_uses_its_time_not_the_scan_time(self):
        self.issue["updated_at"] = self.date(20)
        self.pull["head"]["sha"] = "new-commit"
        self.assertEqual(self.activity(day=21), self.start + timedelta(days=20))

    def test_human_label_changes_are_activity(self):
        event = {"event": "labeled", "created_at": self.date(20),
                 "actor": {"login": "contributor"}, "label": {"name": "soft-stale"}}
        self.assertEqual(self.activity([event]), self.start + timedelta(days=20))


if __name__ == "__main__":
    unittest.main()
