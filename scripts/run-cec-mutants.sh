#!/usr/bin/env bash
#
# Run cargo-mutants over the PURE DECISION MODULES of `cec/` (tv-shell-cec) and
# report the result — caught / missed / unviable / timeout, plus every surviving
# mutant by name.
#
# WHY. Mutation discipline on this crate is manual today and has already failed
# twice: three tests in jedwards1230/tv-shell#514 were vacuous until a hand-run
# mutation caught them (and fixing them exposed a real bug — `Rx` reset the whole
# tx-error run, making the "no rx traffic in the same window" condition dead
# code), and inverting a crate's central rule in jedwards1230/tv-shell#462 left
# all 88 tests passing. A discipline that has failed twice should be automated.
#
# SCOPE lives in .cargo/mutants.toml — glob-based, so the four modules still in
# flight (ownership.rs, action.rs, health.rs, failover.rs) are picked up as each
# PR lands instead of being named as paths that do not exist yet. Read that file
# for which modules are deliberately OUT of scope and why.
#
# THREE OUTCOMES, KEPT DISTINCT. This is the whole point of the script, and the
# reason it is not two lines of YAML with `continue-on-error`:
#
#   ran, surviving mutants found  -> report the numbers, exit 0 (advisory)
#   ran, none survived            -> report, exit 0
#   did not run / examined nothing / output unparsable -> exit 1, HARD
#
# "We could not measure" must never look like "we measured and it was fine".
# `continue-on-error` cannot tell those apart — it reports green on a SKIP as
# readily as on a failure, which is exactly jedwards1230/tv-shell#469, where a
# gated leg was wired to nothing and reported green for months. The zero-files
# check below is that trap in its purest form, so it is a hard failure.
#
# ADVISORY FOR NOW. Surviving mutants do not fail this script yet: the baseline
# has to be established before it can be a threshold. Promote it by failing on
# `missed > 0` (or on a ratchet) once the numbers are known and the survivors
# have been triaged.
#
# RUNTIME. Every mutant is a separate rebuild, so the run is dominated by rustc
# and scales with cores. It is run with `--jobs` set to the machine's core count
# (override with MUTANTS_JOBS) rather than cargo-mutants' serial default.
# `minimum_test_timeout` in .cargo/mutants.toml is the guard that keeps that from
# turning healthy mutants into spurious TIMEOUTs on a loaded box — if timeouts
# ever appear under parallelism and not at `--jobs 1`, raise that floor rather
# than accepting a flaky gate.
#
# USAGE
#   scripts/run-cec-mutants.sh            # run from the workspace root
#   MUTANTS_OUTPUT=/tmp/x scripts/run-cec-mutants.sh
#   MUTANTS_JOBS=1 scripts/run-cec-mutants.sh   # force the serial behaviour

set -euo pipefail

# The modules the gate INTENDS to cover. Kept here as well as in the globs so
# the summary can say which ones were found and which were absent: a module
# missing because its PR has not landed yet is expected and fine, while a module
# missing because someone RENAMED it is a coverage hole — and a hole nobody sees
# is how a gate ends up measuring less every month while still reporting green.
INTENDED_MODULES=(
  cec/src/state.rs
  cec/src/protocol.rs
  cec/src/ownership.rs
  cec/src/action.rs
  cec/src/health.rs
  cec/src/failover.rs
)

OUT_DIR=${MUTANTS_OUTPUT:-mutants-output}
SUMMARY=${GITHUB_STEP_SUMMARY:-/dev/stdout}
# The runner's ACTUAL core count, not a hardcoded guess: `ubuntu-latest` has 4
# today and a developer box has more, and either number would be wrong on the
# other. Falls back to 1 if nproc is unavailable, which is the old behaviour.
JOBS=${MUTANTS_JOBS:-$(nproc 2>/dev/null || echo 1)}

for tool in cargo jq; do
  command -v "$tool" >/dev/null || { echo "::error::$tool is required"; exit 1; }
done
cargo mutants --version >/dev/null 2>&1 || {
  echo "::error::cargo-mutants is not installed — install it before running this gate"
  exit 1
}

echo "== Files cargo-mutants will examine =="
# --list-files resolves the config's globs against the real tree, so this is the
# authoritative answer to "what is actually in scope right now" — not a guess
# from the glob list.
if ! examined=$(cargo mutants -p tv-shell-cec --list-files 2>&1); then
  echo "::error::cargo mutants --list-files failed — the scope could not be resolved," \
       "so nothing below can be trusted"
  printf '%s\n' "$examined"
  exit 1
fi
printf '%s\n' "$examined"

file_count=$(printf '%s\n' "$examined" | grep -c '[^[:space:]]' || true)
if [ "$file_count" -eq 0 ]; then
  echo "::error::cargo-mutants would examine ZERO files. A gate that measures nothing" \
       "reports green forever (jedwards1230/tv-shell#469). Check examine_globs in" \
       ".cargo/mutants.toml against the real paths under cec/src/."
  exit 1
fi

echo
echo "== Intended modules: found vs absent =="
present_rows=""
for m in "${INTENDED_MODULES[@]}"; do
  if printf '%s\n' "$examined" | grep -qxF "$m"; then
    echo "  found:  $m"
    present_rows="${present_rows}| \`$m\` | examined |"$'\n'
  elif [ -f "$m" ]; then
    # On disk but NOT examined: the globs and the tree disagree. That is a
    # silent coverage hole, not a pending PR, so it is a hard failure.
    echo "::error::$m exists on disk but is NOT examined — examine_globs in" \
         ".cargo/mutants.toml no longer matches the tree"
    exit 1
  else
    echo "  absent: $m (expected while its PR is still in flight)"
    present_rows="${present_rows}| \`$m\` | absent — not yet landed |"$'\n'
  fi
done

echo
echo "== Running cargo-mutants (scope: .cargo/mutants.toml, --jobs $JOBS) =="
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

# The run's own exit status is NOT the gate: cargo-mutants exits non-zero when
# mutants survive, which is an advisory condition here. What matters is whether
# it produced a parsable outcomes.json — checked below. A failure to BUILD is
# caught by that same check, because no outcomes file appears.
set +e
cargo mutants -p tv-shell-cec --no-times --jobs "$JOBS" -o "$OUT_DIR"
mutants_status=$?
set -e
echo "cargo-mutants exited $mutants_status"

json="$OUT_DIR/mutants.out/outcomes.json"
if [ ! -f "$json" ]; then
  echo "::error::cargo-mutants produced no outcomes.json at $json — it did not run to" \
       "completion. Failing: 'we could not measure' must not read as 'we measured and" \
       "it was fine'."
  exit 1
fi

if ! jq -e . "$json" >/dev/null 2>&1; then
  echo "::error::$json is not parsable JSON — refusing to report a result from output" \
       "that could not be read"
  exit 1
fi

total=$(jq -r '.total_mutants // "null"' "$json")
caught=$(jq -r '.caught  // "null"' "$json")
missed=$(jq -r '.missed  // "null"' "$json")
unviable=$(jq -r '.unviable // "null"' "$json")
timeout=$(jq -r '.timeout // "null"' "$json")

for pair in "total_mutants:$total" "caught:$caught" "missed:$missed" \
            "unviable:$unviable" "timeout:$timeout"; do
  name=${pair%%:*}
  value=${pair#*:}
  case "$value" in
    ''|null|*[!0-9]*)
      echo "::error::could not read a numeric '$name' out of $json (got '$value')" \
           "— failing rather than reporting an unread number"
      exit 1
      ;;
  esac
done

if [ "$total" -eq 0 ]; then
  echo "::error::cargo-mutants generated ZERO mutants from $file_count file(s). The scope" \
       "resolved but nothing was measured — the #469 trap. Failing."
  exit 1
fi

survivors=$(jq -r '
  .outcomes[]
  | select(.summary == "MissedMutant")
  | .scenario.Mutant
  | "\(.file): \(.function.function_name // "?") -> \(.replacement // "?")"
' "$json")

echo
echo "== Result: $total mutants — $caught caught, $missed missed, $unviable unviable, $timeout timeout =="
[ -n "$survivors" ] && printf '%s\n' "$survivors"

{
  echo "## cargo-mutants — \`cec/\` decision modules"
  echo ""
  echo "**Advisory (non-blocking).** Surviving mutants are reported, not enforced," 
  echo "until the baseline is agreed. The job still FAILS hard if the run examined"
  echo "nothing or could not be parsed — see jedwards1230/tv-shell#469."
  echo ""
  echo "| Outcome | Count |"
  echo "|---------|-------|"
  echo "| Mutants generated | $total |"
  echo "| Caught | $caught |"
  echo "| **Missed (survived)** | **$missed** |"
  echo "| Unviable (did not build) | $unviable |"
  echo "| Timeout | $timeout |"
  echo ""
  echo "Run with \`--jobs $JOBS\`. A non-zero **Timeout** count on a run that is"
  echo "clean at \`--jobs 1\` means the parallel load squeezed the per-mutant"
  echo "budget — raise \`minimum_test_timeout\` in \`.cargo/mutants.toml\` rather"
  echo "than living with a flaky gate."
  echo ""
  echo "### Scope — intended modules"
  echo ""
  echo "| Module | Status |"
  echo "|--------|--------|"
  printf '%s' "$present_rows"
  echo ""
  echo "Files examined: $file_count. Out-of-scope modules (\`kernel/\`, \`ipc.rs\`,"
  echo "\`main.rs\`, \`notify.rs\`, \`backend.rs\`, \`config.rs\`) are I/O, not decisions —"
  echo "mutating them measures the fakes rather than the rules. See \`.cargo/mutants.toml\`."
  echo ""
  if [ "$missed" -gt 0 ]; then
    echo "### Surviving mutants"
    echo ""
    echo '```'
    printf '%s\n' "$survivors"
    echo '```'
  else
    echo "### Surviving mutants"
    echo ""
    echo "None — every generated mutant was caught."
  fi
} >> "$SUMMARY"

echo
echo "OK: cargo-mutants ran and was measured. Surviving mutants are advisory on this job."
