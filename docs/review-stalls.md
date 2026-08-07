# Review stalls

The implementation↔review loop converges mechanically: a pull request is done when its
required checks are green and no reified finding is open. When instead the loop stops making
progress, it escalates to the maintainer rather than iterating quietly. This file is the
durable record of what is currently stuck, so a loop that ran out of ideas stays visible
instead of being absorbed into another round.

The convergence rule, the blocking criterion and the bounded stagnation brake are decided in
[`docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md`](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md) and enforced
by `.github/scripts/check-review-loop.js`.

## What counts as a stall

The loop state is SHA-256 over canonical JSON containing normalized changed-file tuples, open
reified finding IDs and non-successful required job names. A stall is either of:

- **A repeated state.** The current hash equals an earlier round's hash: the round changed
  nothing that matters to convergence.
- **No monotonic progress.** The blocking set `{red required checks} ∪ {open reified
  findings}` has not shrunk strictly for `REVIEW_LOOP_STAGNATION_LIMIT` rounds (3 by default).

For a journal that began without verified genesis authorization, synthetic `GENESIS` is one open
reified finding in that set. Its state is persisted separately from Markdown ledger findings as
the digest-bound `unauthenticatedAnchor` boolean on each new journal record. This special ID does
not enter the ledger's `F-\d{3,}` namespace or its retired-ID reservation; a verified genesis
directive closes it through the ordinary authorization verifier.

The brake measures state, not elapsed time or token budget. A slow round that shrinks the
blocking set is progress; a fast round that does not is not.

The accepted brake has two residual accounting limitations. Alternating between different
blocking sets can reset the non-shrinking streak even when the loop makes no lasting progress.
Also, records created by the legacy pull-request path used every observed red check while the
trusted path uses only its explicitly named required jobs, so a migrated journal can compare
blocking counts drawn from different domains. These are advisory limitations of the accepted
ADR 0008 design; neither changes the current convergence rule.

## How an entry is opened

CI fails the pull request with the exact finding or check that is stuck and refuses to pass
until the stall is recorded here naming that pull request. An agent does not decide to
escalate and does not decide to keep going: continuing to iterate after a stall without this
record is prohibited. Write the entry in the same session the stall is detected.

An entry names what is stuck, not a narrative of the rounds. If the stuck item is a reified
finding, give its ID and the check that fails. If it is a red required check, name the check.

This is enforced, not merely asked for: the checker parses the `## Open` table below and
requires a row for the exact pull request whose `Stuck on` cell names **every** current blocker
as an exact token. Separate multiple blockers with `<br>`; commas and semicolons are ordinary
characters inside a blocker name. Thus `F-0010` does not name `F-001`. A
mention of the pull request elsewhere in
this file, or a row under `Cleared`, does not satisfy it. Naming the pull request without
naming what stuck it would have told the maintainer nothing.

The journal's unkeyed digest chain detects accidental corruption and casual editing that does not
rebuild the chain. It does not authenticate records against another repository workflow, which
can use the same App identity, recompute the digests, and manufacture convergence on a history in
which no finding was ever reified. A deleted record is detected when a surviving descendant names
it. An actor with repository write access can delete the newest record and the evaluator reads the
shorter prefix as though the missing round never happened. Terminal truncation is not detected:
it can erase the record that established reification, after which the shorter authentic prefix
may converge with that finding advisory. This dismissed F-002 limitation remains tracked by
[`docs/review-loop-spike.md`](review-loop-spike.md).

Bounded-history anchor: terminal truncation can erase reification history and enable advisory convergence.

## Recovering a terminal malformed journal record

Journal serialization prevents records written after the delimiter-escaping fix from persisting
a raw `-->` inside their JSON. A malformed record written before that change still fails closed;
the fix is forward-only and does not rewrite existing issue comments.

The malformed record must be terminal. It must be the newest review-loop journal comment and no
surviving descendant may name its digest. Before deleting it, pass every earlier configured-App
journal comment, excluding only the candidate, through the default-branch evaluator's
`verifyJournal` with the repository's exact configured identity. Recover the candidate's complete
JSON record as well: its carried digest must verify, its observation must immediately follow the
surviving head, and its declared `priorDigest` must equal that head's digest. Before deletion,
restore the pull request's current review ledger to the raw pre-projection ledger recorded in the
candidate's digest-bound `ledgerFindings` field. This carries forward findings introduced only by
the malformed record without asking the recovery verifier to invert the evaluator's lossy
projection.

The comparison covers the fields the Markdown ledger represents: `id`, `round`, `reifiedBy`,
parser-derived `reified`, `state`, and `resolution`. The adapter derives `evidence` by parsing
`resolution`, so the resolution comparison pins the evidence the ledger can carry.
`disposalError` exists only in the separate evaluator `findings` projection and is not a Markdown
ledger column. A legacy candidate without `ledgerFindings` exposes only its projected values. A
parser-reachable projection may be replayed directly, but a terminal-malformed legacy record from
a failed disposition is not recoverable by this procedure: its projected `reified: true` /
`state: open` values retain a non-reifying `reifiedBy`, while the Markdown parser derives
`reified` from `reifiedBy` and therefore can never reproduce that combination. The verifier must
refuse it. Do not keep rewriting or refetching the ledger; leave the comment in place and escalate
the permanently unreachable recovery candidate to the maintainer. No non-whitespace content may
follow the recovered frame.

Failed-disposition refusal anchor: stop recovery and escalate to the maintainer.

The refusal anchor check guarantees the anchor's exact text, uniqueness, and position immediately
after the failed-disposition refusal paragraph. Positive prose assertions also require that
paragraph to identify the legacy candidate's projected values, state that the failed disposition
is not recoverable and the verifier must refuse it, forbid continued ledger rewriting or
refetching, and require maintainer escalation. They do not exclude contradictory retry advice
inside that paragraph.

The downloaded comment array must be chronological by both `created_at` and comment ID. When the
API provides nested `performed_via_github_app` attribution, it is authoritative; a conflicting
top-level `app_slug` or `app_id` is refused. These checks harden the operator gate's input
attribution and terminality assumptions inside the already documented bounded App-identity threat
model; they do not establish a new provenance guarantee.

The default-branch `check-review-loop-recovery.js` performs all of those checks mechanically. Use
a clean temporary directory, substitute the repository, pull request and candidate issue-comment
ID, and run exactly:

```sh
recovery_repo=jsunyermias/keeplin
recovery_pull=198
recovery_candidate_comment=123456789
recovery_dir=$(mktemp -d)
recovery_default=$(gh api "repos/$recovery_repo" --jq .default_branch)
recovery_repository_id=$(gh api "repos/$recovery_repo" --jq .id)
recovery_workflow_id=$(gh variable get CI_WORKFLOW_ID --repo "$recovery_repo")
gh api --method GET "repos/$recovery_repo/contents/.github/scripts/check-review-loop.js" -f ref="$recovery_default" --jq .content | base64 --decode > "$recovery_dir/check-review-loop.js"
gh api --method GET "repos/$recovery_repo/contents/.github/scripts/check-review-loop-recovery.js" -f ref="$recovery_default" --jq .content | base64 --decode > "$recovery_dir/check-review-loop-recovery.js"
gh api "repos/$recovery_repo/pulls/$recovery_pull" > "$recovery_dir/pull.json"
gh api --paginate --slurp "repos/$recovery_repo/issues/$recovery_pull/comments?per_page=100" | jq 'add' > "$recovery_dir/comments.json"
node "$recovery_dir/check-review-loop-recovery.js" \
  --pull "$recovery_dir/pull.json" \
  --comments "$recovery_dir/comments.json" \
  --candidate-comment-id "$recovery_candidate_comment" \
  --repository-id "$recovery_repository_id" \
  --workflow-id "$recovery_workflow_id" \
  --app-slug github-actions \
  --app-id 15368
```

`repositoryId` comes from the repository API's `id`; `workflowId` comes from the repository's
`CI_WORKFLOW_ID` variable. `appSlug` (`github-actions`) and `appId` (`15368`) come from the
`config` object in the default-branch `.github/workflows/review-loop-evaluator.yml`; if that
configuration changes, use its exact current values rather than the literals above. The pull API
response supplies the current ledger, and the paginated issue-comment API supplies the complete
comment history. The command's `findingsSource` labels `findings` as either the candidate's
`raw pre-projection ledger snapshot` (records with `ledgerFindings`) or its `legacy evaluator
projection` (records without it); `projectedFindings` is the evaluator output kept separately for
diagnosis. The command always reports the candidate's `unauthenticatedAnchor` value and emits
`null` for a legacy candidate that omits the field. The flag is digest-bound historical state, not
a Markdown ledger field, so it is preserved in recovery output but excluded from the
`ledgerFindings` semantic comparison.
For a raw snapshot, if the command reports that the ledger is not semantically
identical, restore every raw finding from `findings` in the pull-request ledger and fetch
`pull.json` again. A failed-disposition legacy projection is the permanently unreachable case
described above, so repeated restoration cannot make it pass and requires maintainer escalation.

Do not delete anything unless the verifier exits zero. A zero exit proves that the current ledger
matches the candidate snapshot identified by `findingsSource` and establishes an authentic verifying prefix.
It proves that prefix from the remaining comments only within the journal's bounded, unkeyed
threat model, and also establishes predecessor continuity. Keep its JSON output with the recovery
evidence.

After that verification, delete only the terminal malformed comment and rerun the affected CI
evaluation. The evaluator derives the round again from the current pull request, checks and
authorizations, and the round is then re-recorded as the next observation. If the malformed
record is not terminal, the remaining prefix does not verify, or the claimed observation cannot
be established, do not delete anything: restore the missing evidence or escalate to the
maintainer. Deleting a non-terminal record leaves a surviving descendant whose predecessor link
cannot verify and does not recover the pull request.

Disposition authorizations have a separate strict time boundary. GitHub's API timestamps have
one-second granularity, and a directive issued in the same second as the intervening observation
is not strictly after it, so the evaluator rejects it. Reissue the directive in a later second.

## How an entry is cleared

A stall is the maintainer's to resolve, and there are exactly three exits:

1. **Fix what fails** — the reified finding's check goes green, the blocking set shrinks, and
   the loop resumes normally.
2. **Dismiss the finding with a cited reason** — a priority decision or an accepted ADR. The
   finding moves to `dismissed` in the ledger with that citation and does not reopen unless the
   code in its area changes.
3. **Reclassify it as advisory** — only when it genuinely cannot be reduced to a failing
   check and an independent authorized reference carries the machine-readable reclassification
   directive. It is then recorded, tracked as a follow-up issue, and no longer blocking. That
   authorization requirement is bounded by the truncation limit above, and the "only" holds no
   further: it is enforced while the record establishing reification survives. An actor with
   repository write access who deletes the newest records reaches a shorter authentic prefix in
   which the finding was never reified, and can file it advisory there with no authorization at
   all.

Recording a stall here does not make the pull request mergeable, and neither does a green CI
run on an unrelated commit: convergence still requires zero open reified findings. Move the
entry to Cleared with the exit that was taken and a link to it.

## Open

| Detected | Pull request | Stuck on | Rounds without progress | Exit taken |
|---|---|---|---|---|
| 2026-08-05 | [keeplin-srv#114](https://github.com/jsunyermias/keeplin-srv/pull/114) | F-001<br>F-002<br>F-003<br>F-004<br>F-005<br>F-006<br>F-007<br>F-008<br>F-010<br>F-011<br>F-012<br>F-013<br>F-014<br>F-018<br>GENESIS | 3 | — |
| 2026-08-05 | [keeplin-srv#116](https://github.com/jsunyermias/keeplin-srv/pull/116) | GENESIS | 3 | |
| 2026-08-07 | [keeplin-srv#126](https://github.com/jsunyermias/keeplin-srv/pull/126) | F-002 | 3 | — |

[keeplin-srv#116](https://github.com/jsunyermias/keeplin-srv/pull/116) is stuck on `GENESIS`, and it
is the companion of [keeplin#217](https://github.com/jsunyermias/keeplin/pull/217), which
**implements** the exit. It escalated on the repeated-state brake rather than the non-shrinking one:
successive evaluations ran against an unchanged tree, so the loop state repeated and the blocking
set was never going to shrink in between.

The reason it cannot open its own gate is mechanical. The evaluator runs from the **default
branch**, so the authorization path that would close `GENESIS` is the one on `main` — not the one
that pull request adds. The exit becomes available to the *next* pull request once the pair merges,
and to this one never.

Recorded so a reader who finds an implementation blocked by the defect it fixes knows it was
understood rather than overlooked. It is also why the row above, for keeplin-srv#114, still has no
exit: that pull request is merged, and re-evaluating a closed pull request is a route this decision
deliberately left undefined.

[keeplin-srv#121](https://github.com/jsunyermias/keeplin-srv/pull/121) is stuck on `GENESIS` alone:
its journal opened with `unauthenticatedAnchor: true`, `findings: []` and `blocking: 1`. It
escalated on the repeated-state brake at observation 2, not on the non-shrinking one — the second
evaluation was triggered by an edit to the pull-request body, which changes nothing the loop state
covers, so the hash repeated byte for byte.

Unlike keeplin-srv#116 above, this pull request **can** reach its own exit. The evaluator runs from
the default branch, and the ADR 0015 Option C authorization path that closes `GENESIS` is now
merged there via [keeplin#217](https://github.com/jsunyermias/keeplin/pull/217) and
[keeplin-srv#116](https://github.com/jsunyermias/keeplin-srv/pull/116). The exit is therefore
exit 2 of the three below, taken through the procedure in
[`docs/review-directives.md`](review-directives.md): a verified genesis directive from a qualifying
principal. That directive was issued, at
[#issuecomment-5202730982](https://github.com/jsunyermias/keeplin-srv/pull/121#issuecomment-5202730982),
and the five reified findings of review round two were dismissed with cited reasons at
[#issuecomment-5203525444](https://github.com/jsunyermias/keeplin-srv/pull/121#issuecomment-5203525444).
It is worth recording that the very first pull request opened after the exit landed still had to
escalate in order to use it, which is the per-pull-request cost that
[keeplin#220](https://github.com/jsunyermias/keeplin/issues/220) exists to remove.

<a id="keeplin-srv-121-evaluation-gap"></a>

**The exit was taken, but its effect was never observed.** `keeplin-srv#121` merged on 2026-08-06
at 18:48 UTC without the `Review loop converged` check ever having been evaluated on its merge
candidate, `3862005`. No `Review loop converged` check exists on `3862005` at all: its checks are
`Check, Test & Lint` and `Knowledge graph up to date`, both successful. The results that opened
this stall are the two failures on the first commit, `43517cb`. Evaluator run `31095205483`
evaluated `3862005` at 10:55:51 UTC but exited on the invalid ledger identifier `REV-121-06` before
publishing `Review loop converged`. Whether the evaluator ran on any other commit between
`43517cb` and the merge was not established here. That evaluator run preceded the repository's
last observed workflow run at 16:52 UTC by roughly six hours. No repository workflow run was
created after 16:52 UTC, and the merge itself triggered no build of `main`. Consequently, the
merged `main` tree remains unverified by CI, and any other pull request merged during that window
likewise lacks checks on its merge commit; verification of those merged trees remains pending.

What that leaves unproven is narrow and should not be overstated. The convergence condition was
never computed on the merged tree: not that it would have failed. Its inputs were green when the
merge happened — `Check, Test & Lint` and `Knowledge graph up to date` both succeeded on
`3862005`, the genesis anchor was authorized, and no reified finding remained open. Independent
review is a separate and conjunctive requirement, and it was met: two rounds by a different model
family, recorded on the pull request. What is missing is the machine's own verdict, and a
convergence claim that no machine computed is exactly the kind of claim this file exists to keep
visible.

There is no remedy inside this decision. Re-evaluating a closed pull request is the route left
undefined above, so this row moves to Cleared naming the exit that was taken, with the gap stated
rather than implied.

[keeplin-srv#126](https://github.com/jsunyermias/keeplin-srv/pull/126) escalated at journal
observation 4 (run ID `31165307830`, head `2a9e60d`) because the blocking set had not shrunk for
three rounds. Its size remained one throughout, but its membership changed completely:
observations 1–3 each had `GENESIS` as their sole blocker, while observation 4 records
`unauthenticatedAnchor: false`, a verified genesis directive and `F-002` as the sole open finding.
The genesis gate therefore closed and real progress was made even though the blocking-set size
did not shrink.

Like keeplin-srv#116 above, this pull request is blocked by the defect it fixes. The evaluator
runs from the default branch, so the contract that would close `F-002` is `main`'s rather than
this branch's. Unlike #116, two exits exist. The maintainer can re-run the evaluator for CI
workflow run `31164587869`, to which `F-002`'s recorded check run ID `92822415400` is bound, or
#126 can merge, after which its derived-check contract removes this class of problem for later
pull requests. An automated attempt to re-run the evaluator was refused with
`403 Resource not accessible by integration`; the re-run is therefore a maintainer action, not
an overlooked automated step. This is recorded for the same reason as #116: a reader who finds
an implementation blocked by the defect it fixes can see that the mechanism was understood
rather than overlooked.

## Cleared

| Detected | Pull request | Stuck on | Exit | Resolution |
|---|---|---|---|---|
| 2026-08-06 | [keeplin-srv#121](https://github.com/jsunyermias/keeplin-srv/pull/121) | GENESIS | authorized genesis | The verified [genesis directive](https://github.com/jsunyermias/keeplin-srv/pull/121#issuecomment-5202730982) closed `GENESIS`; the unresolved candidate-evaluation gap is [recorded here](#keeplin-srv-121-evaluation-gap). |
| 2026-08-03 | [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198) | F-002 | dismissed | Accepted [ADR 0008](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md) bounds the claim; the three-probe follow-up is [tracked](review-loop-spike.md). |
| 2026-08-03 | [keeplin#198](https://github.com/jsunyermias/keeplin/pull/198) | F-008<br>F-013 | resolved | Default-branch isolation and verified-disposal tests pass. |
