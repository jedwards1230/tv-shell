#!/usr/bin/env bash
#
# Fail the build if a C toolchain or a system C library enters a crate's build.
#
# WHY THIS IS A SEPARATE SCRIPT FROM assert-pure-rust-tls.sh
# ----------------------------------------------------------
# It shares that script's mechanism — `cargo tree --invert` over a banned list,
# where SUCCESS IS THE FAILURE CONDITION — and deliberately not its subject.
# `assert-pure-rust-tls.sh` asserts which TLS *provider* resolves: its name, its
# banned list (aws-lc-rs / native-tls / openssl-sys) and above all its
# remediation message ("pass default-features = false and pin rustls to ring")
# are about crypto-provider feature selection. This script asserts something
# else entirely: that no C compiler, no binding generator and no system library
# is involved in building a crate at all. Folding both into one file would give
# it two unrelated jobs and one remediation message that is wrong for half its
# callers. They cross-reference each other instead.
#
# THE INVARIANT
# -------------
# `cec/Cargo.toml` chose `linux-cec` over the alternatives specifically because
# `linux-cec-sys` ships PRE-GENERATED bindings — no bindgen, no libclang, no
# system C library — which preserves the workspace rule that a default build
# needs no system C libraries (the rule `daemon/Cargo.toml` fought for, and the
# reason the `cec` CI job installs no apt packages at all).
#
# Nothing asserted it. A future `linux-cec` bump that switched to bindgen would
# stay GREEN on a GitHub runner that happens to have libclang installed, and
# then fail on the deploy box. The invariant held only by accident.
#
# TWO HALVES, BOTH REQUIRED — each catches what the other misses:
#
#   graph  a build-dependency appearing at all (bindgen, cmake, pkg-config…),
#          including one that would only *fail* on a machine lacking the tool.
#          Catches it before anything is linked, and catches build-time-only
#          tooling that leaves no trace in the finished binary.
#   ldd    what actually got linked. A crate can bind a system library through
#          a path the graph check does not name — so this half is an ALLOWLIST,
#          not a denylist. `rust.yml`'s `cec-mcp` job greps for
#          `libcec|libp8-platform`, which is a denylist and would sail straight
#          past a new, unanticipated system library. That is exactly the
#          regression class here, so this half enumerates what is ALLOWED and
#          fails on anything else.
#
# USAGE
#   scripts/assert-no-system-c.sh graph -p tv-shell-cec
#   scripts/assert-no-system-c.sh ldd target/release/tv-shell-cec
#
# See also: scripts/assert-pure-rust-tls.sh (the TLS/crypto provider gate).

set -euo pipefail

# Crates whose presence means "a C toolchain or a system library entered the
# build". `cargo tree -i <crate>` exits 0 when the crate IS in the graph, so the
# check is inverted: success is the failure condition.
#
# ON `cc`, WHICH IS THE INTERESTING ONE. `cc` is the build-dependency that
# actually invokes a C compiler, so on this invariant's own terms it belongs
# here — and it is genuinely absent from `tv-shell-cec`'s graph today
# (`cargo tree -p tv-shell-cec -i cc` does not match). It is NOT banned anyway,
# on purpose: `cc` is a build-dep of plenty of harmless pure-Rust-facing crates
# (`ring` is the workspace's own live example — see assert-pure-rust-tls.sh,
# which states the same caveat from the other direction), so banning it turns
# this gate into a tripwire that fires on dependency changes that do not
# threaten the invariant, and a gate people learn to work around is worse than
# no gate. The things that actually break the deploy — a binding generator, a
# build system, a system-library probe, a `-sys` crate for a library that must
# be installed — are all named explicitly below, and the `ldd` half catches
# anything that slips past the names.
BANNED=(
  bindgen      # runs libclang at build time; the exact regression this guards
  clang-sys    # bindgen's libclang binding, in case only the lower crate appears
  cmake        # a build system the target box would have to ship
  pkg-config   # probes for a system library, i.e. one must be installed
  system-deps  # pkg-config's declarative front end, same implication
  vcpkg        # the same probe on the other platforms
  libcec-sys   # v1's static-libcec path; must never reach the v2 crate (#179)
  libudev-sys  # C libudev; what libcec-sys needed and this crate must not
  openssl-sys  # any path to a system OpenSSL
  libclang-sys # older name for the libclang binding
)

usage() {
  cat >&2 <<'EOF'
usage: scripts/assert-no-system-c.sh graph <cargo tree args...>
       scripts/assert-no-system-c.sh ldd <binary>
EOF
  exit 2
}

assert_graph() {
  local args=("$@")
  [ ${#args[@]} -gt 0 ] || usage

  echo "Asserting no C-toolchain/system-library crate in: cargo tree ${args[*]}"
  echo "(cargo tree includes build- AND dev-dependencies, so a test-only bindgen counts too)"

  local failed=0 crate out preflight

  # PREFLIGHT — resolve the tree ONCE before inverting anything.
  #
  # This closes a hole the inverted mechanism has by construction, and which is
  # easy to miss: `cargo tree -p typo --invert bindgen` fails with "package ID
  # specification `typo` did not match any packages" — a message about the -p
  # SPEC, not about the banned crate. Matched loosely, that reads as "bindgen is
  # absent", and every crate in the banned list then "passes" against a package
  # that does not exist. The whole gate reports green having examined nothing.
  # Measured, not theorised: `assert-no-system-c.sh graph -p no-such-crate`
  # exited 0 before this block existed. (assert-pure-rust-tls.sh had the same
  # hole; it is fixed there too.)
  #
  # Resolving the tree first makes a bad package spec, a broken manifest or a
  # lockfile mismatch a hard failure here, where it names itself.
  if ! preflight=$(cargo tree "${args[@]}" 2>&1); then
    echo "::error::cargo tree ${args[*]} does not resolve — failing here rather than" \
         "letting every banned-crate check read as 'absent'"
    printf '%s\n' "$preflight"
    exit 1
  fi

  for crate in "${BANNED[@]}"; do
    # Capture combined output so a genuine cargo failure can be told apart from
    # a clean "not in the graph". Without that distinction a broken manifest, a
    # lockfile mismatch or a network error would ALSO exit non-zero and the gate
    # would silently pass — a check that cannot fail is not a check. Carried
    # over verbatim in intent from assert-pure-rust-tls.sh.
    if out=$(cargo tree "${args[@]}" --invert "$crate" 2>&1); then
      echo "::error::banned crate '$crate' is in the dependency graph"
      printf '%s\n' "$out" | head -30
      failed=1
    elif printf '%s' "$out" | grep -q "specification \`$crate\` did not match any packages"; then
      # Match the CRATE's own name in the message, never a bare "did not match
      # any packages" — see the preflight note above for why the loose form is
      # how a gate ends up examining nothing.
      echo "  ok: $crate absent"
    else
      echo "::error::cargo tree failed unexpectedly while checking '$crate' —" \
           "treating this as a failure rather than a pass"
      printf '%s\n' "$out"
      exit 1
    fi
  done

  if [ "$failed" -ne 0 ]; then
    cat >&2 <<'EOF'

A C-toolchain or system-library crate reached the dependency graph.

This breaks the invariant that a default build of this workspace needs no
system C libraries: no bindgen, no libclang, no cmake, no pkg-config-probed
library. It is why the `cec` job in .github/workflows/rust.yml installs no apt
packages, and why the crate builds identically on a runner and on the deploy
box. A dependency that switched to generated bindings would stay green on a
runner that happens to have libclang and fail only on hardware.

Fix it at the dependency, not here — prefer a version/feature that keeps
pre-generated bindings (cec/Cargo.toml records why `linux-cec` was chosen on
exactly this basis). If a system library is genuinely unavoidable, that is a
design decision to surface in review, with the CI job's apt step and this
banned list changed together in the same PR.
EOF
    exit 1
  fi

  echo "OK: no C toolchain and no system-library crate in the graph."
}

# The base set every dynamically linked Linux binary has. Confirmed by running
# `ldd` on a real release build of tv-shell-cec: linux-vdso, libgcc_s, libm,
# libc, and the loader — nothing else. glibc < 2.34 split libpthread/libdl/librt
# out of libc rather than merging them, so those three are allowed as well; they
# are the same C library under different sonames, not an additional dependency,
# and allowing them keeps this from failing on an older container image for a
# reason that has nothing to do with the invariant.
ALLOWED_SONAMES='^(linux-vdso\.so\.1|libc\.so\.6|libm\.so\.6|libgcc_s\.so\.1|libpthread\.so\.0|libdl\.so\.2|librt\.so\.1|ld-linux.*\.so\.[0-9]+)$'

assert_ldd() {
  local bin=${1:-}
  [ -n "$bin" ] || usage
  if [ ! -x "$bin" ]; then
    echo "::error::$bin is not an executable file — build it before asserting on it"
    exit 1
  fi

  local out
  # A static binary makes ldd exit non-zero with "not a dynamic executable".
  # That satisfies the invariant trivially, so it is a pass — but it is reported,
  # not swallowed, and every OTHER ldd failure is still a hard failure. Anything
  # less would let a broken ldd invocation read as a clean allowlist.
  if ! out=$(ldd "$bin" 2>&1); then
    if printf '%s' "$out" | grep -q 'not a dynamic executable'; then
      echo "OK: $bin is statically linked — it links no system library at all."
      return 0
    fi
    echo "::error::ldd failed on $bin — treating this as a failure rather than a pass"
    printf '%s\n' "$out"
    exit 1
  fi

  echo "ldd $bin:"
  printf '%s\n' "$out"

  # The library NAME is what the allowlist judges — never the resolved path,
  # which differs between a Debian container and the Arch deploy box.
  local sonames
  sonames=$(printf '%s\n' "$out" \
    | sed -e 's/(0x[0-9a-f]*)$//' -e 's/=>.*$//' \
    | tr -d '\t' | sed -e 's#.*/##' -e 's/^ *//' -e 's/ *$//' \
    | grep -v '^$')

  if [ -z "$sonames" ]; then
    echo "::error::parsed no shared-library names out of ldd's output — refusing to" \
         "report a pass from an empty list"
    exit 1
  fi

  local failed=0 name
  while IFS= read -r name; do
    if printf '%s' "$name" | grep -Eq "$ALLOWED_SONAMES"; then
      echo "  ok: $name (base C runtime)"
    else
      echo "::error::$bin links '$name', which is not in the allowed base set"
      failed=1
    fi
  done <<<"$sonames"

  if [ "$failed" -ne 0 ]; then
    cat >&2 <<'EOF'

The binary links a system library beyond the base C runtime.

This is an ALLOWLIST rather than a denylist by design: the existing
`libcec|libp8-platform` grep in rust.yml's cec-mcp job would not have noticed a
new, unanticipated system library, and that is precisely the regression this
guards. If the new link is intended, add its soname to ALLOWED_SONAMES here,
install it in the CI job, and say in the PR why the crate now needs a system
library it was designed not to need.
EOF
    exit 1
  fi

  echo "OK: links nothing beyond the base C runtime."
}

mode=${1:-}
[ -n "$mode" ] || usage
shift || true
case "$mode" in
  graph) assert_graph "$@" ;;
  ldd)   assert_ldd "$@" ;;
  *)     usage ;;
esac
