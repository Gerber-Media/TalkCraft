# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
#
# Exports:
#   ROOT                    workspace root
#   VERSION                 TalkCraft's version, TALKCRAFT_MINOR.<commits since the fork> from
#                           packaging/version.env (override: TALKCRAFT_VERSION); also exported as
#                           TALKCRAFT_VERSION so the binaries report the same number
#   DIST                    output directory for release artifacts (default: $ROOT/dist/release)
#   TALKCRAFT_BUILD_SHA    git commit (recorded in the macOS Info.plist and the job summary)
#   TALKCRAFT_BUILD_DATE   UTC build date, YYYY-MM-DD (AppStream release date)
#   CARGO_TARGET_DIR        cargo's target dir (default: $ROOT/target)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# The version is set in exactly one place, packaging/version.env (the same numbers the app shows
# in About; crates/engine/build.rs computes it the same way). The crates keep the SoundCraft
# version in Cargo.toml, so upstream merges stay simple.
talkcraft_version() {
  local minor base count
  minor="$(sed -n 's/^TALKCRAFT_MINOR=//p' "$ROOT/packaging/version.env")"
  base="$(sed -n 's/^TALKCRAFT_FORK_BASE=//p' "$ROOT/packaging/version.env")"
  # CI checks out shallowly by default: the count needs the history back to the fork.
  if ! git -C "$ROOT" cat-file -e "$base^{commit}" 2>/dev/null && [ "$(git -C "$ROOT" rev-parse --is-shallow-repository 2>/dev/null)" = "true" ]; then
    git -C "$ROOT" fetch --quiet --unshallow >&2 || true
  fi
  if count="$(git -C "$ROOT" rev-list --count "$base..HEAD" 2>/dev/null)"; then
    echo "$minor.$count"
  fi
}

VERSION="${TALKCRAFT_VERSION:-$(talkcraft_version)}"
if [ -z "$VERSION" ]; then
  # Never ship a made-up number: every installer would carry the same one.
  echo "error: could not count the commits since the fork (packaging/version.env); fetch the full history or set TALKCRAFT_VERSION" >&2
  exit 1
fi
export VERSION
export TALKCRAFT_VERSION="$VERSION"
# For builds that see neither git nor this environment (the Flatpak sandbox): gitignored.
echo "$VERSION" > "$ROOT/packaging/talkcraft-version"

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

if [ -z "${TALKCRAFT_BUILD_SHA:-}" ]; then
  TALKCRAFT_BUILD_SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
fi
export TALKCRAFT_BUILD_SHA
export TALKCRAFT_BUILD_DATE="${TALKCRAFT_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}

# Copy licence and readme files that exist into a package directory.
copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE LICENSE-MIT LICENSE-APACHE NOTICE ATTRIBUTION.md COPYRIGHT; do
    if [ -f "$ROOT/$f" ]; then cp "$ROOT/$f" "$dest/"; fi
  done
}


# Portable SHA-256 of a file (prints just the hash).
sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
