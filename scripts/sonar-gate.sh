#!/usr/bin/env bash
set -euo pipefail

PROJECT_KEY="raihan0x1_0x1-ai-memory"
BRANCH="main"
WAIT_SECS=0
SHA=""

usage() {
  cat <<'EOF'
Usage: ./scripts/sonar-gate.sh [--sha <sha>] [--branch <branch>] [--wait <seconds>]

Checks SonarCloud Quality Gate for an exact commit SHA on a branch (fail-closed).
Polls up to --wait seconds if SonarCloud has not yet analyzed --sha.
EOF
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --sha)
      SHA="${2:-}"
      shift 2
      ;;
    --branch)
      BRANCH="${2:-}"
      shift 2
      ;;
    --wait)
      WAIT_SECS="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$SHA" ]]; then
  SHA="$(git rev-parse HEAD)"
elif RESOLVED_SHA="$(git rev-parse --verify "${SHA}^{commit}" 2>/dev/null)"; then
  SHA="$RESOLVED_SHA"
fi

if ! [[ "$WAIT_SECS" =~ ^[0-9]+$ ]]; then
  echo "ERROR: --wait must be a non-negative integer (got '$WAIT_SECS')." >&2
  exit 1
fi

python3 - "$PROJECT_KEY" "$BRANCH" "$SHA" "$WAIT_SECS" <<'PY'
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

project_key, branch, target_sha, wait_raw = sys.argv[1:5]
wait_secs = int(wait_raw)
base = "https://sonarcloud.io/api"


def api_get(path: str, params: dict[str, str]) -> dict:
    qs = urllib.parse.urlencode(params)
    url = f"{base}{path}?{qs}"
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.loads(resp.read().decode("utf-8"))


def print_failures(qg: dict) -> None:
    failing = [
        f"{c.get('metricKey')}={c.get('actualValue')} (threshold {c.get('errorThreshold')})"
        for c in qg.get("conditions", [])
        if c.get("status") != "OK"
    ]
    print(
        f"ERROR: SonarCloud Quality Gate failed on {target_sha[:7]} ({branch}): {', '.join(failing)}",
        file=sys.stderr,
    )

    try:
        issues_res = api_get(
            "/issues/search",
            {
                "componentKeys": project_key,
                "branch": branch,
                "statuses": "OPEN,CONFIRMED",
                "inNewCodePeriod": "true",
                "types": "BUG,VULNERABILITY",
                "ps": "50",
            },
        )
        issues = issues_res.get("issues", [])
        if issues:
            print("Open BUG / VULNERABILITY issues in new code:", file=sys.stderr)
            for issue in issues:
                comp = issue.get("component", "").split(":", 1)[-1]
                line = issue.get("line", "?")
                print(
                    f"  - [{issue.get('type')}/{issue.get('severity')}] {issue.get('rule')} {comp}:{line} -> {issue.get('message')}",
                    file=sys.stderr,
                )
    except Exception as exc:
        print(f"WARNING: Could not fetch SonarCloud issue details ({exc})", file=sys.stderr)

    try:
        hs_res = api_get(
            "/hotspots/search",
            {
                "projectKey": project_key,
                "branch": branch,
                "status": "TO_REVIEW",
                "ps": "50",
            },
        )
        hotspots = hs_res.get("hotspots", [])
        if hotspots:
            print("Security hotspots TO_REVIEW:", file=sys.stderr)
            for hs in hotspots:
                comp = hs.get("component", "").split(":", 1)[-1]
                line = hs.get("line", "?")
                print(
                    f"  - [{hs.get('vulnerabilityProbability')}] {hs.get('ruleKey')} {comp}:{line} -> {hs.get('message')}",
                    file=sys.stderr,
                )
    except Exception as exc:
        print(f"WARNING: Could not fetch SonarCloud hotspot details ({exc})", file=sys.stderr)

    dash_q = urllib.parse.urlencode({"id": project_key, "branch": branch})
    print(f"See: https://sonarcloud.io/dashboard?{dash_q}", file=sys.stderr)


deadline = time.monotonic() + wait_secs
last_seen = "none"
last_err: Exception | None = None
printed_wait = False

while True:
    try:
        ana_res = api_get(
            "/project_analyses/search",
            {"project": project_key, "branch": branch, "ps": "5"},
        )
        analyses = ana_res.get("analyses", [])
        latest_rev = analyses[0].get("revision", "") if analyses else ""
        if latest_rev:
            last_seen = latest_rev[:7]

        if latest_rev == target_sha:
            qg = api_get(
                "/qualitygates/project_status",
                {"projectKey": project_key, "branch": branch},
            ).get("projectStatus", {})
            if qg.get("status") == "OK":
                print(f"SonarCloud Quality Gate: OK (rev={target_sha[:7]}, branch={branch})")
                sys.exit(0)
            print_failures(qg)
            sys.exit(1)

        if any(a.get("revision") == target_sha for a in analyses[1:]):
            print(
                f"ERROR: SonarCloud latest analysis on {branch} is already {last_seen}, past target {target_sha[:7]}.",
                file=sys.stderr,
            )
            sys.exit(1)

        last_err = None
    except Exception as exc:
        last_err = exc

    remaining = deadline - time.monotonic()
    if remaining <= 0:
        if last_err is not None:
            print(
                f"ERROR: Could not query SonarCloud for {target_sha[:7]} on {branch}: {last_err}",
                file=sys.stderr,
            )
        else:
            print(
                f"ERROR: SonarCloud has not analyzed {target_sha[:7]} on branch {branch} yet "
                f"(latest analyzed: {last_seen}). Push first or pass --wait 240.",
                file=sys.stderr,
            )
        sys.exit(1)

    if not printed_wait:
        print(
            f"Waiting up to {wait_secs}s for SonarCloud analysis of {target_sha[:7]} on {branch} "
            f"(latest: {last_seen})..."
        )
        printed_wait = True
    time.sleep(min(5.0, remaining))
PY
