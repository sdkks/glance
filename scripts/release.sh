#!/bin/sh
# Local-only release: commit the stamped trio and create an annotated tag.
set -eu
cd "$(git rev-parse --show-toplevel)"
bump=${1:-}
exact=${2:-}
branch=$(git symbolic-ref --quiet --short HEAD) || {
    echo 'Release requires a branch, not detached HEAD.' >&2
    exit 1
}
dirty=$(git status --porcelain --untracked-files=no | awk '
    { path=substr($0,4) }
    path!="Cargo.toml" && path!="Cargo.lock" && path!="CHANGELOG.md" { print }
')
if [ -n "$dirty" ]; then
    printf 'Refusing release with unrelated tracked changes:\n%s\n' "$dirty" >&2
    exit 1
fi
old_head=$(git rev-parse HEAD)
snap=$(mktemp -d "${TMPDIR:-/tmp}/glance-release.XXXXXX")
restore=false
release_head=
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    if [ "$restore" = true ]; then
        # Commit uses a private index. Roll back only our own new HEAD, never
        # an intervening commit, then restore the caller’s original file bytes.
        if [ -n "$release_head" ]; then
            git update-ref HEAD "$old_head" "$release_head" || {
                echo 'HEAD changed concurrently; manual recovery required.' >&2
                exit 1
            }
        fi
        cp -p "$snap/Cargo.toml" "$snap/Cargo.lock" "$snap/CHANGELOG.md" .
        echo 'Release failed; stamped files restored; no release commit or tag retained.' >&2
    fi
    rm -f "$snap/Cargo.toml" "$snap/Cargo.lock" "$snap/CHANGELOG.md" "$snap/index" "$snap/index.lock" "$snap/message"
    rmdir "$snap"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
cp -p Cargo.toml Cargo.lock CHANGELOG.md "$snap/"
restore=true
sh scripts/bump-version.sh "$bump" "$exact"
version=$(make --no-print-directory version)
tag="v$version"
if git show-ref --verify --quiet "refs/tags/$tag"; then
    printf 'Tag %s already exists.\n' "$tag" >&2
    exit 1
fi
# Keep the real index (including partial staging of the trio) intact on failure.
GIT_INDEX_FILE="$snap/index" git read-tree "$old_head"
GIT_INDEX_FILE="$snap/index" git add -- Cargo.toml Cargo.lock CHANGELOG.md
for file in Cargo.toml Cargo.lock CHANGELOG.md; do
    if GIT_INDEX_FILE="$snap/index" git diff --cached --quiet -- "$file"; then
        printf 'Release must change all three stamped files; %s is unchanged.\n' "$file" >&2
        exit 1
    fi
done
printf 'chore(release): release %s\n' "$tag" > "$snap/message"
if ! GIT_INDEX_FILE="$snap/index" git commit -F "$snap/message"; then
    exit 1
fi
release_head=$(git rev-parse HEAD)
git tag -a "$tag" -m "Glance $tag"
# Once the tag exists the release is complete; do not undo a published local tag
# if index synchronization fails. The index can be refreshed without losing files.
restore=false
git reset -q HEAD -- Cargo.toml Cargo.lock CHANGELOG.md
printf '\nReleased %s on %s; nothing pushed.\nReview the commit and tag, then run:\n  git push origin %s %s\n' "$tag" "$branch" "$branch" "$tag"
