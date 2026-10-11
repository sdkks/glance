#!/bin/sh
# Write the tagged changelog body, or the caller's fallback, to the notes file.
set -eu
changelog=$1
version=${2#v}
notes=$3
fallback=$4
awk -v version="$version" '
    /^## \[/ {
        if (found) exit
        heading = "## [" version "]"
        if ($0 == heading || index($0, heading " ") == 1) found = 1
        next
    }
    found {
        if ($0 ~ /^[[:space:]]*$/) {
            if (body != "") blanks = blanks $0 "\n"
        } else {
            body = body blanks $0 "\n"
            blanks = ""
        }
    }
    END { printf "%s", body }
' "$changelog" > "$notes"
if [ ! -s "$notes" ]; then
    printf '%s\n' "$fallback" > "$notes"
    echo 'Release notes: embedded summary fallback (section missing or empty).'
else
    echo 'Release notes: tagged CHANGELOG section.'
fi
