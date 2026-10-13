#!/usr/bin/env bash
# A scheduled workflow's result as a GitHub issue (docs/DEEP_TESTING.md D0,
# Q8): a failed run opens an issue titled after the workflow, or comments on
# the one already open; a run that passes closes it. Nobody has to look at
# the Actions tab to find out the nightly is red.
#
# Run as the last job of a scheduled workflow, with:
#   NEEDS     the `needs` context as JSON (`${{ toJSON(needs) }}`)
#   WORKFLOW  the workflow's name
#   RUN_URL   the run's page
#   GH_TOKEN  a token with `issues: write`
#   DRY_RUN   set to print what would be done instead of doing it
set -euo pipefail

LABEL="nightly-failure"
TITLE="Scheduled run failing: ${WORKFLOW}"

failed=$(python3 -c 'import json, sys
print(",".join(k for k, v in json.loads(sys.argv[1]).items() if v.get("result") == "failure"))' "$NEEDS")

gh_do() {
    if [ -n "${DRY_RUN:-}" ]; then
        printf 'would run: gh'
        printf ' %q' "$@"
        printf '\n'
    else
        gh "$@"
    fi
}

open=$(gh issue list --label "$LABEL" --state open --json number,title \
    --jq ".[] | select(.title == \"$TITLE\") | .number" | head -n1)

if [ -n "$failed" ]; then
    body="Failed: \`${failed//,/\`, \`}\` — ${RUN_URL}"
    if [ -n "$open" ]; then
        gh_do issue comment "$open" --body "Still failing. $body"
    else
        gh_do label create "$LABEL" --color d73a4a \
            --description "A scheduled workflow is failing" --force
        gh_do issue create --title "$TITLE" --label "$LABEL" --body "$body"
    fi
elif [ -n "$open" ]; then
    gh_do issue comment "$open" --body "Passing again — ${RUN_URL}"
    gh_do issue close "$open"
else
    echo "$WORKFLOW passed; no issue open."
fi
