#!/bin/sh
# Bump only the package version, refresh its lock entry, and stamp release notes.
# Explicit VERSION retries converge; no commit, tag, or network access is needed.
set -eu
cd "$(git rev-parse --show-toplevel)"
bump=${1:-}
exact=${2:-}
if [ -n "$bump" ] && [ -n "$exact" ]; then
    echo 'Choose BUMP or VERSION, not both.' >&2
    exit 1
fi
choice=${exact:-${bump:-patch}}
case "$choice" in
    major|minor|patch) ;;
    *) printf '%s\n' "$choice" | grep -Eq '^(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})$' || {
        echo 'Expected major, minor, patch, or stable x.y.z (up to nine digits per component).' >&2
        exit 1
    } ;;
esac

snap=$(mktemp -d "${TMPDIR:-/tmp}/glance-bump.XXXXXX")
changed=false
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    if [ "$changed" = true ]; then
        cp -p "$snap/Cargo.toml" "$snap/Cargo.lock" "$snap/CHANGELOG.md" .
        echo 'Bump failed; Cargo.toml, Cargo.lock and CHANGELOG.md restored.' >&2
    fi
    rm -f "$snap/Cargo.toml" "$snap/Cargo.lock" "$snap/CHANGELOG.md" "$snap/manifest" "$snap/changelog" "$snap/version"
    rmdir "$snap"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
cp -p Cargo.toml Cargo.lock CHANGELOG.md "$snap/"
awk -v choice="$choice" -v result="$snap/version" '
    /^[ \t]*\[/ {
        package=($0 ~ /^[ \t]*\[package\][ \t]*(#.*)?$/)
        if (package) packages++
    }
    package && /^[ \t]*version[ \t]*=/ {
        versions++
        if ($0 !~ /^[ \t]*version[ \t]*=[ \t]*"(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})"[ \t]*(#.*)?$/) { bad=1; next }
        old=$0; sub(/^[^"]*"/,"",old); sub(/".*$/,"",old)
        split(old,v,".")
        if (choice=="major") ver=sprintf("%.0f.0.0",v[1]+1)
        else if (choice=="minor") ver=sprintf("%.0f.%.0f.0",v[1],v[2]+1)
        else if (choice=="patch") ver=sprintf("%.0f.%.0f.%.0f",v[1],v[2],v[3]+1)
        else ver=choice
        sub(/"[0-9]+\.[0-9]+\.[0-9]+"/,"\"" ver "\"")
    }
    { print }
    END {
        if (packages!=1 || versions!=1 || bad) {
            print "Expected exactly one literal stable [package] version." > "/dev/stderr"
            exit 1
        }
        print ver > result
    }
' Cargo.toml > "$snap/manifest"
version=$(awk '{print}' "$snap/version")
# Prepare notes before changing any project file. Exact-version retries do not
# stamp a second section or consume new Unreleased notes for an existing release.
awk -v ver="$version" -v today="$(date -u +%Y-%m-%d)" '
    FNR==NR { if (index($0,"## [" ver "] - ")==1) stamped=1; next }
    /^## \[Unreleased\]$/ {
        unreleased++
        print
        if (!stamped) { print ""; print "## [" ver "] - " today }
        next
    }
    { print }
    END { if (unreleased!=1) { print "Expected exactly one Unreleased heading." > "/dev/stderr"; exit 1 } }
' CHANGELOG.md CHANGELOG.md > "$snap/changelog"
changed=true
cp "$snap/manifest" Cargo.toml
# Metadata refreshes the local package entry without compiling native libraries
# or updating the resolved dependency versions.
cargo metadata --offline --format-version 1 > /dev/null
cp "$snap/changelog" CHANGELOG.md
changed=false
printf 'Version bumped to %s; inspect Cargo.toml, Cargo.lock and CHANGELOG.md before committing.\n' "$version"
