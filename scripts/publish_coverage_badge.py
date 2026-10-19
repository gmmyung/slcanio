"""Publish a Shields endpoint JSON file to the coverage-badge branch."""

import base64
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

BRANCH = "coverage-badge"
BADGE_PATH = "coverage.json"


def github(method: str, path: str, payload: dict | None = None) -> dict | None:
    data = json.dumps(payload).encode() if payload is not None else None
    request = urllib.request.Request(
        f"https://api.github.com/repos/{os.environ['GITHUB_REPOSITORY']}/{path}",
        data=data,
        method=method,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
            "Content-Type": "application/json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if method == "GET" and error.code == 404:
            return None
        raise RuntimeError(f"GitHub API {method} {path} failed: HTTP {error.code}") from error


def main() -> None:
    report = json.loads(Path(sys.argv[1]).read_text())
    percent = float(report["data"][0]["totals"]["lines"]["percent"])
    badge = {
        "schemaVersion": 1,
        "label": "coverage",
        "message": f"{percent:.1f}%",
        "color": "brightgreen" if percent >= 90 else "green" if percent >= 75 else "yellow" if percent >= 60 else "red",
    }
    content = (json.dumps(badge, separators=(",", ":")) + "\n").encode()

    if github("GET", f"git/ref/heads/{BRANCH}") is None:
        github("POST", "git/refs", {"ref": f"refs/heads/{BRANCH}", "sha": os.environ["GITHUB_SHA"]})

    current = github("GET", f"contents/{BADGE_PATH}?ref={BRANCH}")
    if current and base64.b64decode(current["content"]) == content:
        print(f"Coverage badge is current: {badge['message']}")
        return

    update = {
        "message": f"Update coverage badge to {badge['message']}",
        "content": base64.b64encode(content).decode(),
        "branch": BRANCH,
    }
    if current:
        update["sha"] = current["sha"]
    github("PUT", f"contents/{BADGE_PATH}", update)
    print(f"Published coverage badge: {badge['message']}")


if __name__ == "__main__":
    main()
