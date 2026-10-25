---
name: land
description: >-
  Prepare and publish a zview-ui pull request, monitor its checks, and verify
  completion after the user merges it on GitHub. Invoke only when the user
  explicitly requests landing or invokes /land, not for review, preparation,
  passing checks, or skill installation.
disable-model-invocation: true
metadata:
  delta-action: land
---

# Land zview-ui changes through a pull request

## Contract

An explicit invocation supplies permission to prepare the intended changes,
create a branch and commits, publish that branch, and open a pull request.
Proceed without asking for the same permission again. Stop for genuine blockers
or unclear scope.

The user performs the final merge on GitHub. Do not merge automatically, enable
auto-merge, push directly to the destination branch, or bypass repository rules.
Publishing a branch, opening a PR, or obtaining green checks is not completion.
Finish successfully only after verifying that GitHub reports the PR as merged
and that its resulting commit is included in the intended destination.

Apply this workflow only to zview-ui. The expected repository is
`zano-systems/zview-ui`, with `main` as the destination. Verify those facts each
time rather than applying them to another repository.

## 1. Inspect the change and destination

- Read applicable `AGENT.md` / `AGENTS.md`, `CONTRIBUTING.md`, submission
  templates, and `.github/workflows/ci.yml`. Recheck policy when resuming.
- Inspect working-tree status, staged and unstaged diffs, untracked files,
  current branch, existing commits, and configured remotes. Separate the
  requested work from unrelated edits. Never indiscriminately stage everything.
- Verify the publication remote against GitHub repository identity and access.
  Never publish through the `local` remote. Pause if the intended repository,
  destination, or change scope cannot be established.
- Verify `git`, `gh`, authentication, runtime versions, and required components.
  Do not print credentials, change authentication, or install tools globally.
- Read current GitHub rulesets, branch protection, merge restrictions, required
  checks, and review requirements. A missing legacy branch-protection record
  does not mean that no ruleset applies. Do not change protections.
- At setup time, `main` required linear history and PRs permitting only rebase
  merges. Verify current rules before advising the user how to merge.
- Check for an existing PR for this change before creating another one.

The project's closed-contributions notice does not prohibit its documented
maintainer workflow. Follow that workflow for this maintainer-requested work;
do not use it as an invitation for outside contributions.

## 2. Prepare a dedicated branch and commits

Fetch the verified source remote's destination and inspect divergence. Create a
new, descriptive, unused branch for the requested change when starting from the
destination branch. Reuse an existing dedicated branch and PR when resuming.
Preserve intended uncommitted changes and already-reviewed commits.

Pause on **all merge or rebase conflicts**. Describe the conflicting files and
operation and wait for the user's decision; do not resolve even apparently
clear conflicts automatically. Do not reset, clean, stash, discard files,
overwrite another person's changes, or rewrite the destination's history.

Inspect the final diff, stage only in-scope paths or hunks, and create focused
Conventional Commits using explicit messages. Use non-interactive Git commands,
including `GIT_EDITOR=true git commit -m "..."`. Preserve existing authorship
and configured signing. Stop on signing or hook errors rather than disabling
them.

Source: `CONTRIBUTING.md` defines Conventional Commits, separate branches and
PRs, and intentional versioning. Do not bump the version for every change.
When a version change is in scope, keep `Cargo.toml` and `Cargo.lock` aligned;
document breaking interfaces and migration steps when applicable. Recheck for
any new agreements, human-authored submission requirements, or templates and
fulfil their actual conditions.

## 3. Verify before submitting

Run from the repository root:

| Check | Source establishing the invocation and target |
| --- | --- |
| `cargo fmt --check` | `.github/workflows/ci.yml`, Check formatting step |
| `cargo test --locked` | `Cargo.toml` defines the Rust package and test dependencies; `src/main.rs` includes the test module; the CI Run Rust tests step uses this Cargo invocation with additional test-runner arguments |
| `cargo clippy --locked --all-targets -- -D warnings` | `.github/workflows/ci.yml`, Run Clippy step |

These satisfy the local formatting, tests, and lint checks required by
`CONTRIBUTING.md`; there is no documented docs-only exemption. Also run
`git diff --check` and checks relevant to the actual change, such as validating
edited documentation links. Report manual visual and platform checks not run.
Do not let a representative local run replace required remote checks.

At setup, CI used macOS 15, Rust 1.99.0 with rustfmt and Clippy, Python 3.12.10,
and ZView revision `40c85a79ab88491e4050e84e03d94b6790c1dca6`. Re-read
`.github/workflows/ci.yml`, `Cargo.toml`, and `Cargo.lock` for current versions.
Python dependencies are pinned in `.github/requirements-ci.txt`.

The complete recording verification belongs to the CI workflow. If reproducing
it locally, use its matching isolated Python environment and exact commands:

- `python -m unittest discover -s . -p test_export_recording.py -v`
  — source: `.github/workflows/ci.yml`, Test recording exporter step, and
  `test_export_recording.py`.
- Generate `local/bench-with-maps.json` using the complete Generate heap-map
  integration fixture step in `.github/workflows/ci.yml`; the exporter argument
  definitions are in `export_recording.py`. Do not overwrite committed fixtures.
- `cargo test --locked -- --include-ignored`
  — source: `.github/workflows/ci.yml`, Run Rust tests including opt-in
  integration tests step. `src/recording.rs` and `src/tests.rs` identify the
  ignored tests' Python and heap-map prerequisites.

Use `ZVIEW_PYTHON` to select an existing interpreter with ZView installed when
needed; source: `src/recording.rs`. Do not assume the default Python matches CI
or contains ZView. If a mandatory local check cannot run, report the exact
blocker and pause rather than claiming success or silently skipping it.
Do not weaken checks, alter lockfiles unintentionally, or install system tools
to make a submission proceed.

## 4. Publish the branch and open the PR

- Push the dedicated branch normally, with its upstream set to the verified
  publication remote. Do not push to `main`.
- Open the PR explicitly against the verified base and head using
  `gh pr create --repo zano-systems/zview-ui --base main --head <branch>
  --title <title> --body-file <file>`. Replace placeholders with actual values.
  Use a temporary local body file, not an interactive editor. Do not use
  `--dry-run` as a safety mechanism; it may still push.
- Include a factual summary, actual check results, manual checks not performed,
  and any applicable migration notes. Follow any current submission template.
  Do not claim tests or reviews that have not happened.
- Record the PR URL, branch, and exact head SHA. If a publish/create command
  errors, inspect remote state before retrying to avoid duplicate PRs.

## 5. Monitor the latest revision and handle failures

Use `gh pr view <number> --repo zano-systems/zview-ui --json
url,state,headRefOid,baseRefName,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup`
to establish the current revision and state. Use `gh pr checks <number>
--repo zano-systems/zview-ui` to inspect checks; `--required` alone can be empty
even when the project's CI must still pass.

Monitor with `gh pr checks <number> --repo zano-systems/zview-ui --watch
--fail-fast --interval 20` only in a bounded terminal execution. When the
execution bound expires, report pending status and resume on a later turn.
Do not start an unattended infinite watcher or imply monitoring continues while
no execution is running.

Require the CI workflow's `Rust and recording tests (macOS)` job and all
additional checks required by current rules to succeed for the current PR
revision. Correlate run event, PR identity, run attempt, and commit or PR test
merge revision with the current head. A prior head's success is insufficient.
Pending, failed, missing, cancelled, skipped, or unverifiable required checks
are not green. Inspect the actual workflow run when a check rollup is unclear.

On failure:

- Obtain relevant diagnostics with `gh run view <run-id>
  --repo zano-systems/zview-ui --log-failed`, targeting the actual failing run.
- Report the PR, head SHA, failing job and step, useful log excerpt, and known
  reproduction command. Distinguish code failures from infrastructure failures.
- Pause for corrections and another review; do not bypass failures, invent a
  fix, or repeatedly rerun without diagnosing the cause.
- Once reviewed corrections are supplied, inspect the diff and review evidence,
  rerun applicable local checks, and update this same PR. Do not treat earlier
  approval as review of newly changed code.

Prefer a normal push. A rewrite is permitted only when necessary after an
explicitly approved amend or rebase of this PR branch. Fetch and compare the
remote tip with the last known tip; stop if another person has updated it.
Use an explicit `--force-with-lease=refs/heads/<branch>:<expected-sha>` bound to
that known tip, never plain `--force` or a lease on the destination branch.
After any push, restart verification for the new head.

## 6. Ask for the human merge and verify completion

Once all required checks have passed for the current revision, applicable
review requirements are satisfied, and GitHub reports the PR is mergeable:

1. Recheck the head SHA and current destination rules.
2. Tell the user that the PR is ready, provide its URL and verified commit,
   and ask them to merge it on GitHub using the currently allowed method.
3. Report **awaiting user merge**, not success. Do not run a merge command.

When the user reports merging, or a subsequent state check detects it, query
`gh pr view <number> --repo zano-systems/zview-ui --json
state,mergedAt,mergeCommit,baseRefName,headRefOid,url`. Verify `state` is
`MERGED`, the base is correct, and the resulting commit is reachable from
the remote destination, using a fresh fetch and Git ancestry verification.
Rebase merging may give destination commits different SHAs from the PR head.

If the head changes before merging, return to verification. If the PR is
closed unmerged, checks fail, review is outstanding, or destination inclusion
cannot be verified, report that the change has not been confirmed landed and
state the blocker. Never treat PR closure as a merge.

On verified completion, report the PR URL, destination, resulting commit,
and checks validated. Leave branch deletion and unrelated workspace cleanup
to an explicit request.
