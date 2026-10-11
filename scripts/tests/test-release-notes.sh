#!/bin/sh
# Hermetic checks of the same notes writer used by the release workflow.
set -eu
cd "$(dirname "$0")/../.."
for tool in awk mktemp diff; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "SKIP: release notes tests require $tool."
        exit 0
    fi
done
work=$(mktemp -d /tmp/glance-release-notes-test.XXXXXX)
trap 'rm -rf "$work"' EXIT HUP INT TERM
cat > "$work/changelog" <<'EOF'
# Changelog

## [Unreleased]

- Future change

## [0.1.0] - 2026-10-11

### Added

- First release

## [0.0.9]

- Previous release
EOF
printf '### Added\n\n- First release\n' > "$work/expected"
sh scripts/release-notes.sh "$work/changelog" v0.1.0 "$work/notes" 'Fallback summary'
diff -u "$work/expected" "$work/notes"
# Trim leading/trailing whitespace-only lines, retaining interior blank lines.
printf '## [0.1.0]\n\n \t\n### Added\n\n- First release\n\n \t\n\n' > "$work/changelog"
sh scripts/release-notes.sh "$work/changelog" v0.1.0 "$work/notes" 'Fallback summary'
diff -u "$work/expected" "$work/notes"
# CRLF headings, body and section boundary produce the same LF-only notes.
printf '## [0.1.0]\r\n\r\n### Added\r\n\r\n- First release\r\n\r\n## [0.0.9]\r\n- Previous release\r\n' > "$work/changelog"
sh scripts/release-notes.sh "$work/changelog" v0.1.0 "$work/notes" 'Fallback summary'
diff -u "$work/expected" "$work/notes"
printf 'Fallback summary\n' > "$work/expected"
printf '## [0.1.0]\n\n \t\n## [0.0.9]\n- Previous release\n' > "$work/changelog"
sh scripts/release-notes.sh "$work/changelog" v0.1.0 "$work/notes" 'Fallback summary'
diff -u "$work/expected" "$work/notes"
printf '## [0.1.01]\n- Not the requested version\n## [Unreleased]\n- Future change\n' > "$work/changelog"
sh scripts/release-notes.sh "$work/changelog" v0.1.0 "$work/notes" 'Fallback summary'
diff -u "$work/expected" "$work/notes"
# A short version must not match a longer version's prefix.
printf '## [0.10]\n- Not the requested version\n' > "$work/changelog"
sh scripts/release-notes.sh "$work/changelog" v0.1 "$work/notes" 'Fallback summary'
diff -u "$work/expected" "$work/notes"
# The input is genuinely absent; require success as well as fallback content.
if sh scripts/release-notes.sh "$work/missing-changelog" v0.1.0 "$work/notes" 'Fallback summary'; then
    diff -u "$work/expected" "$work/notes"
else
    echo 'FAIL: missing changelog must exit 0.' >&2
    exit 1
fi
echo 'Release notes regression checks passed'
