#!/bin/sh
# Focused deployment-version parser regression tests; no native builds needed.
set -eu
cd "$(dirname "$0")/../.."
work=$(mktemp -d /tmp/glance-floor-test.XXXXXX)
trap 'rm -rf "$work"' EXIT HUP INT TERM
cat > "$work/otool" <<'EOF'
#!/bin/sh
cat "$2"
EOF
chmod +x "$work/otool"
PATH="$work:$PATH"
export PATH
printf ' cmd LC_BUILD_VERSION\n minos 11.0\n sdk 26.4\n' > "$work/current"
printf ' cmd LC_VERSION_MIN_MACOSX\n version 10.15\n sdk 26.4\n' > "$work/legacy"
sh scripts/assert-macos-floor.sh 11.0 "$work/current" "$work/legacy"
for version in 11.0.1 11.1 12.0 26.0; do
    printf ' cmd LC_BUILD_VERSION\n minos %s\n' "$version" > "$work/newer"
    if sh scripts/assert-macos-floor.sh 11.0 "$work/newer"; then
        echo "Accepted incompatible version: $version" >&2; exit 1
    fi
done
printf ' cmd LC_BUILD_VERSION\n sdk 26.4\n' > "$work/missing"
if sh scripts/assert-macos-floor.sh 11.0 "$work/missing"; then
    echo 'Accepted missing version' >&2; exit 1
fi
# A compatible first architecture must not hide a too-new second architecture.
cat "$work/current" "$work/newer" > "$work/fat"
if sh scripts/assert-macos-floor.sh 11.0 "$work/fat"; then
    echo 'Accepted incompatible fat slice' >&2; exit 1
fi
echo 'macOS deployment floor regression checks passed'
