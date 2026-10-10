#!/bin/sh
# Reject missing/too-new Mach-O deployment versions, including every fat slice.
set -eu
floor=$1
shift
for file do
    otool -l "$file" | awk -v floor="$floor" -v file="$file" '
        function newer(version, limit, v, l, n, m, i) {
            n = split(version, v, "."); m = split(limit, l, ".")
            for (i = 1; i <= n || i <= m; i++) {
                if (v[i] + 0 > l[i] + 0) return 1
                if (v[i] + 0 < l[i] + 0) return 0
            }
            return 0
        }
        $1 == "cmd" { command = $2 }
        (command == "LC_BUILD_VERSION" && $1 == "minos") ||
        (command == "LC_VERSION_MIN_MACOSX" && $1 == "version") {
            seen++; print file ": minimum macOS " $2 " (floor " floor ")"
            if (newer($2, floor)) bad = 1
        }
        END {
            if (!seen || bad) {
                print "Missing or incompatible macOS deployment version: " file > "/dev/stderr"
                exit 1
            }
        }' || exit 1
done
