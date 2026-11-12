"""Apply inactivity labels to open PRs without counting our own updates as activity."""

import hashlib
import json
import os
from datetime import datetime, timedelta, timezone
from urllib.parse import quote
from urllib.request import Request, urlopen


STAGES = ((7, "soft-stale"), (14, "medium-stale"), (21, "hard-stale"))
LABELS = {label for _, label in STAGES}
BOT = "github-actions[bot]"
MARKER = "<!-- tern-projects:pr-inactivity\n"
NOTE = (
    "Inactivity is checked daily: soft-stale after 7 days, medium-stale after 14, "
    "and hard-stale after 21. New activity clears the stale label. "
    "This does not close the PR automatically.\n\n"
)


def timestamp(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def stale_label(last_activity, now):
    for days, label in reversed(STAGES):
        if now - last_activity >= timedelta(days=days):
            return label
    return None


def fingerprint(issue, pull):
    # Commit dates can predate a push; detect head changes independently of them.
    content = [pull["head"]["sha"], issue["title"], issue["body"]]
    return hashlib.sha256(json.dumps(content).encode()).hexdigest()


def activity_time(issue, pull, comments, timeline, review_comments, marker, now):
    last_activity = timestamp(issue["created_at"])
    automation = []
    if marker:
        state = json.loads(marker["body"].split(MARKER, 1)[1].split("\n-->", 1)[0])
        last_activity = max(last_activity, timestamp(state["last_activity"]))
        automation.append(timestamp(marker["updated_at"]))

    for event in timeline:
        own_label = (
            event.get("event") in ("labeled", "unlabeled")
            and (event.get("actor") or {}).get("login") == BOT
            and event.get("label", {}).get("name") in LABELS
        )
        own_comment = marker and event.get("event") == "commented" and event.get("id") == marker["id"]
        if own_label:
            automation.append(timestamp(event["created_at"]))
        elif not own_comment:
            for field in ("created_at", "updated_at", "submitted_at"):
                if event.get(field):
                    last_activity = max(last_activity, timestamp(event[field]))

    activity_comments = [comment for comment in comments if not marker or comment["id"] != marker["id"]]
    for comment in activity_comments + review_comments:
        last_activity = max(last_activity, timestamp(comment["updated_at"]))

    updated = timestamp(issue["updated_at"])
    if not automation or updated > max(automation):
        last_activity = max(last_activity, updated)
    elif marker and state["fingerprint"] != fingerprint(issue, pull):
        # An edit/push racing our writes must not disappear behind our timestamp.
        last_activity = max(last_activity, now)
    return last_activity


class GitHub:
    def __init__(self):
        self.url = os.environ.get("GITHUB_API_URL", "https://api.github.com").rstrip("/")
        self.repo = os.environ["GITHUB_REPOSITORY"]
        self.token = os.environ["GITHUB_TOKEN"]

    def request(self, path, method="GET", data=None):
        request = Request(
            f"{self.url}/repos/{self.repo}/{path}",
            data=json.dumps(data).encode() if data is not None else None,
            method=method,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Accept": "application/vnd.github+json",
                "Content-Type": "application/json",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        with urlopen(request, timeout=30) as response:
            body = response.read()
        return json.loads(body) if body else None

    def pages(self, path):
        page = 1
        while True:
            separator = "&" if "?" in path else "?"
            batch = self.request(f"{path}{separator}per_page=100&page={page}")
            yield from batch
            if len(batch) < 100:
                return
            page += 1


def main():
    github = GitHub()
    existing = {label["name"] for label in github.pages("labels")}
    for (days, label), color in zip(STAGES, ("fbca04", "d93f0b", "b60205")):
        if label not in existing:
            github.request("labels", "POST", {
                "name": label, "color": color,
                "description": f"No PR activity for at least {days} days",
            })

    for summary in github.pages("pulls?state=open&sort=created&direction=asc"):
        number = summary["number"]
        pull = github.request(f"pulls/{number}")
        issue = github.request(f"issues/{number}")
        if pull["state"] != "open" or issue["state"] != "open":
            continue
        comments = list(github.pages(f"issues/{number}/comments"))
        marker = next((comment for comment in comments
                       if comment["user"]["login"] == BOT and MARKER in (comment["body"] or "")), None)
        timeline = list(github.pages(f"issues/{number}/timeline"))
        review_comments = list(github.pages(f"pulls/{number}/comments"))
        now = datetime.now(timezone.utc)
        last_activity = activity_time(issue, pull, comments, timeline, review_comments, marker, now)
        desired = stale_label(last_activity, now)
        current = {label["name"] for label in issue["labels"]} & LABELS

        if desired or marker:
            body = NOTE + MARKER + json.dumps({
                "last_activity": last_activity.isoformat(),
                "fingerprint": fingerprint(issue, pull),
            }, sort_keys=True) + "\n-->"
            # Save the clock before labeling so interrupted runs can resume safely.
            if marker and marker["body"] != body:
                github.request(f"issues/comments/{marker['id']}", "PATCH", {"body": body})
            elif not marker:
                github.request(f"issues/{number}/comments", "POST", {"body": body})

        if desired and desired not in current:
            github.request(f"issues/{number}/labels", "POST", {"labels": [desired]})
        for label in sorted(current - {desired}):
            github.request(f"issues/{number}/labels/{quote(label, safe='')}", "DELETE")
        print(f"PR #{number}: {desired or 'active'} (last activity {last_activity.isoformat()})")


if __name__ == "__main__":
    main()
