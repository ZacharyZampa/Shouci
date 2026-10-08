#!/usr/bin/env bash
# Makes a GitHub release of Shouci for people to download: the version
# raised where it is kept, the checks and tests run, and the disk image
# built. Then, once you say yes, the release commit and tag are pushed and
# the release is published with the disk image attached.
#
#   scripts/release.sh 1.1.0                  # release 1.1.0
#   scripts/release.sh 1.1.0 --draft          # left as a draft to look over on GitHub
#   scripts/release.sh 1.1.0 --notes ~/new.md # your notes, not the commits since the last release
#                                             # (outside the checkout, which must be clean)
#   scripts/release.sh 1.1.0 --yes            # no question (for a script)
#
# Nothing leaves this Mac before the question, and answering no puts the
# version back as it was.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"
PROJECT="macos/Shouci.xcodeproj/project.pbxproj"
VERSION_FILES=(Cargo.toml Cargo.lock "${PROJECT}")

usage() {
  echo "usage: ${0} VERSION [--draft] [--notes FILE] [--yes]" >&2
  exit 2
}

fail() {
  echo "error: $*" >&2
  exit 1
}

VERSION=""
DRAFT=false
NOTES=""
YES=false
while [ $# -gt 0 ]; do
  case "$1" in
    --draft) DRAFT=true ;;
    --notes)
      [ $# -ge 2 ] || usage
      NOTES="$2"
      shift
      ;;
    --yes) YES=true ;;
    -*) usage ;;
    *)
      [ -z "${VERSION}" ] || usage
      VERSION="$1"
      ;;
  esac
  shift
done
[[ "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
TAG="v${VERSION}"

# Put back on the way out, unless the release commit was made.
BUMPED=false
COMMITTED=false
DERIVED=""
NOTES_FILE=""
cleanup() {
  if [ -n "${DERIVED}" ]; then rm -rf "${DERIVED}"; fi
  if [ -n "${NOTES_FILE}" ]; then rm -f "${NOTES_FILE}"; fi
  if ${BUMPED} && ! ${COMMITTED}; then
    git checkout -q -- "${VERSION_FILES[@]}"
    echo "The version is back to ${CURRENT}."
  fi
}
trap cleanup EXIT

# --- Before anything changes

command -v gh >/dev/null 2>&1 || fail "the GitHub CLI is needed: brew install gh"
gh auth status >/dev/null 2>&1 || fail "sign in to GitHub first: gh auth login"
[ "$(git branch --show-current)" = "main" ] || fail "releases are made from main"
[ -z "$(git status --porcelain)" ] || fail "commit or stash your changes first"
[ -z "${NOTES}" ] || [ -f "${NOTES}" ] || fail "no notes file at ${NOTES}"

git fetch -q --tags origin main
git merge-base --is-ancestor origin/main HEAD || fail "origin/main has commits main lacks: pull first"
if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null \
  || [ -n "$(git ls-remote --tags origin "refs/tags/${TAG}")" ]; then
  fail "the tag ${TAG} exists already"
fi
if gh release view "${TAG}" >/dev/null 2>&1; then fail "the release ${TAG} exists already"; fi

# Newer than the last release, whose tag may leave out a .0 (v1.0).
LAST_TAG="$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)"
if [ -n "${LAST_TAG}" ]; then
  last="${LAST_TAG#v}"
  while [ "$(tr -cd . <<<"${last}" | wc -c | tr -d ' ')" -lt 2 ]; do last="${last}.0"; done
  if [ "${last}" = "${VERSION}" ] || [ "$(printf '%s\n' "${last}" "${VERSION}" | sort -V | tail -1)" != "${VERSION}" ]; then
    fail "${VERSION} isn't newer than the last release, ${LAST_TAG}"
  fi
fi

# --- Checks and tests, on the code as it is

echo "==> checks and tests"
scripts/check.sh
macos/scripts/build-core.sh
DERIVED="$(mktemp -d)"
xcodebuild -project macos/Shouci.xcodeproj -scheme Shouci \
  -destination "platform=macOS,arch=$(uname -m)" -derivedDataPath "${DERIVED}" -quiet test

# --- The version, everywhere it's kept: the Rust workspace (and its
# lockfile) and the app, whose build number rises with it.

CURRENT="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml)"
if [ "${CURRENT}" != "${VERSION}" ]; then
  echo "==> version ${CURRENT} → ${VERSION}"
  BUMPED=true
  sed -i '' "s/^version = \"${CURRENT}\"$/version = \"${VERSION}\"/" Cargo.toml
  cargo update -q --workspace --offline
  build="$(sed -n 's/.*CURRENT_PROJECT_VERSION = \([0-9]*\);.*/\1/p' "${PROJECT}" | sort -n | tail -1)"
  sed -i '' \
    -e "s/MARKETING_VERSION = [^;]*;/MARKETING_VERSION = ${VERSION};/" \
    -e "s/CURRENT_PROJECT_VERSION = [0-9]*;/CURRENT_PROJECT_VERSION = $((build + 1));/" \
    "${PROJECT}"
fi

# --- The disk image

echo "==> building the disk image (Apple silicon and Intel)"
macos/scripts/make-dmg.sh "${ROOT}/dist"
DMG="dist/Shouci-${VERSION}.dmg"
[ -f "${DMG}" ] || fail "make-dmg.sh didn't write ${DMG}"
hdiutil verify -quiet "${DMG}"
SHA="$(cut -d' ' -f1 "${DMG}.sha256")"

# --- What the release page says

NOTES_FILE="$(mktemp)"
{
  if [ -n "${NOTES}" ]; then
    cat "${NOTES}"
  elif [ -n "${LAST_TAG}" ]; then
    echo "## What's new"
    echo
    git log --no-merges --reverse --format='- %s' "${LAST_TAG}..HEAD"
  fi
  cat <<EOF

**Install:** open the disk image and drag Shouci to Applications. It needs macOS 14 or later, and runs on Apple silicon and Intel.

**First launch:** there is no paid Apple developer account behind Shouci, so the app is not notarized and macOS blocks it the first time. Open System Settings › Privacy & Security, scroll to the message about Shouci, and choose **Open Anyway**. The dictionary downloads when you first open it.

This download is the app only. The \`shouci\` and \`shouci-tui\` terminal tools come with building from source (see the README).

SHA-256 of \`Shouci-${VERSION}.dmg\`: \`${SHA}\`
EOF
} >"${NOTES_FILE}"

# --- The question, then GitHub

AHEAD="$(git rev-list --count origin/main..HEAD)"
echo
echo "Ready to release Shouci ${VERSION}:"
echo "  disk image  ${DMG}"
if ${BUMPED}; then
  echo "  commit      \"release ${VERSION}\" on main, after ${AHEAD} commit(s) not on GitHub yet"
else
  echo "  commit      none (the version is ${VERSION} already); ${AHEAD} commit(s) not on GitHub yet"
fi
echo "  tag         ${TAG}"
if ${DRAFT}; then echo "  release     a draft, to publish from GitHub"; else echo "  release     published, as the latest"; fi
echo
echo "----- notes -----"
cat "${NOTES_FILE}"
echo "-----------------"
echo

if ! ${YES}; then
  [ -t 0 ] || fail "nothing pushed: run it in a terminal to answer, or pass --yes"
  read -r -p "Push main and ${TAG} to GitHub, and create the release? [y/N] " answer
  if [[ ! "${answer}" =~ ^[Yy] ]]; then
    echo "Nothing was pushed."
    exit 0
  fi
fi

if ${BUMPED}; then git commit -q -m "release ${VERSION}" -- "${VERSION_FILES[@]}"; fi
COMMITTED=true
git tag -a "${TAG}" -m "Shouci ${VERSION}"
if ! git push -q --atomic origin main "${TAG}"; then
  fail "the push failed; the commit and tag are here. Push them with:
  git push --atomic origin main ${TAG}
then run: gh release create ${TAG} ${DMG} ${DMG}.sha256 --title \"Shouci ${VERSION}\" --verify-tag"
fi

# Not an array: macOS's bash 3.2 calls an empty one unbound under `set -u`.
draft=""
if ${DRAFT}; then draft="--draft"; fi
gh release create "${TAG}" "${DMG}" "${DMG}.sha256" --title "Shouci ${VERSION}" \
  --notes-file "${NOTES_FILE}" --verify-tag ${draft:+"${draft}"}
echo "Released: $(gh release view "${TAG}" --json url -q .url)"
