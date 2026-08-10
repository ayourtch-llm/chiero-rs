#!/bin/bash
# What GitHub Actions says about this repository — **without a token, and without lying when
# the quota runs out.**
#
# ⚠️ **This exists because of two mistakes on 2026-08-10, and the second one is the point.**
#
# For two days HANDOFF.md said CI was red, named two candidate causes, and recorded that the
# failing leg could not be identified because *"there is no `gh` CLI on this machine, which is
# why this stops here rather than at an answer."* The repository is **public**: the Actions API
# answers anonymously, and two requests name the failing leg and step. The answer had been one
# `curl` away the whole time. **"I have no way to check X" is a claim about the world and
# deserves a probe rather than a paragraph.**
#
# Then, classifying 53 red runs in a loop, the anonymous quota (**60 requests an hour**) ran out
# and abuse detection tripped. Both answer **HTTP 200** with a JSON body that has no `jobs` key
# — so the parser reported every run as having no failed job, and 53 confident, wrong lines went
# past before anything checked `/rate_limit`. **A quota that answers 200 looks exactly like a
# clean result**, which is `findings: 0` wearing an HTTP status.
#
# So this checks the budget first, refuses to start a sweep it cannot finish, and says which
# requests it spent.
#
#   ./ci-status.sh              the last 10 runs
#   ./ci-status.sh 30           the last 30
#   ./ci-status.sh --why        …and, for the newest failure, the failing leg and step
#
# `GITHUB_TOKEN` is used if it is set (5000/hour instead of 60), and is not required.
set -u

REPO=${REPO:-ayourtch-llm/chiero-rs}
API="https://api.github.com"
AUTH=()
[ -n "${GITHUB_TOKEN:-}" ] && AUTH=(-H "Authorization: Bearer $GITHUB_TOKEN")

get() { curl -sS -m 30 "${AUTH[@]}" "$1"; }

# **The budget, before anything spends it.** A sweep that dies half way leaves a reader with a
# partial answer that looks whole.
budget=$(get "$API/rate_limit")
remaining=$(printf '%s' "$budget" | python3 -c "
import json,sys
try: print(json.load(sys.stdin)['resources']['core']['remaining'])
except Exception: print(-1)
")
if [ "$remaining" -lt 0 ]; then
  echo "cannot read $API/rate_limit — no network, or GitHub is refusing entirely:"
  printf '%s\n' "$budget" | head -c 300
  exit 2
fi

want=10
why=0
for a in "$@"; do
  case "$a" in
  --why) why=1 ;;
  [0-9]*) want=$a ;;
  esac
done
# One request for the list, two more per run inspected by --why.
need=$((1 + why * 2))
if [ "$remaining" -lt "$need" ]; then
  echo "rate limit: $remaining request(s) left, this needs $need."
  echo "  The anonymous limit is 60/hour per IP. Set GITHUB_TOKEN for 5000, or wait."
  echo "  ⚠️ Do not just retry: an exhausted quota answers 200 with a body that has no data,"
  echo "     which reads as an empty result rather than as a refusal."
  exit 3
fi

runs=$(get "$API/repos/$REPO/actions/runs?per_page=$want")
printf '%s' "$runs" | python3 -c "
import json,sys
d=json.load(sys.stdin)
if 'workflow_runs' not in d:
    # The shape that caused the misread: a 200 with no data.
    print('NO DATA in the response — this is a refusal wearing a 200:')
    print(' ', d.get('message','(no message)')[:160])
    raise SystemExit(3)
rs=d['workflow_runs']
if not rs:
    print('no workflow runs at all'); raise SystemExit(0)
for r in rs:
    print(f\"{r['created_at']}  {r['head_sha'][:8]}  {r['status']:<10} {str(r['conclusion']):<9} {r['display_title'][:46]}\")
bad=[r for r in rs if r['conclusion'] not in ('success', None)]
print()
print(f'{len(rs)-len(bad)}/{len(rs)} green in this window' if rs else '')
if bad:
    print(f'newest failure: {bad[0][\"id\"]}  {bad[0][\"head_sha\"][:8]}')
" || exit $?

if [ $why -eq 1 ]; then
  id=$(printf '%s' "$runs" | python3 -c "
import json,sys
d=json.load(sys.stdin)
bad=[r for r in d.get('workflow_runs',[]) if r['conclusion'] not in ('success',None)]
print(bad[0]['id'] if bad else '')
")
  if [ -z "$id" ]; then
    echo
    echo "--why: nothing failed in this window."
  else
    echo
    echo "--why: run $id"
    get "$API/repos/$REPO/actions/runs/$id/jobs" | python3 -c "
import json,sys
d=json.load(sys.stdin)
if 'jobs' not in d:
    print('  NO DATA — a refusal wearing a 200:', d.get('message','')[:120]); raise SystemExit(3)
for j in d['jobs']:
    if j.get('conclusion') == 'failure':
        for s in j.get('steps', []):
            if s.get('conclusion') not in ('success','skipped',None):
                print(f\"  {j['name']}  ->  {s['name']}: {s['conclusion']}\")
"
    # 📌 The step names are the answer. Job *logs* need admin rights on the repository and
    # return 403 anonymously, which is worth knowing before anyone plans around them.
    echo "  (job logs need admin rights — 403 anonymously; the step names are what is readable)"
  fi
fi
