#!/bin/sh
# Retain the unique scratch directory and full listings for review.
set -eu
scratch=$(mktemp -d)
echo "Scratch prefix test: $scratch"
prefix=$scratch/prefix
mkdir -p "$prefix/bin" "$prefix/share/applications"
printf 'unrelated\n' > "$prefix/bin/other-app"
printf 'unrelated\n' > "$prefix/share/applications/other.desktop"
find "$prefix" -type f | sort > "$scratch/before"
cksum "$prefix/bin/other-app" "$prefix/share/applications/other.desktop" > "$scratch/sentinel-checksums"
make install-linux PREFIX="$prefix" DRY_RUN=1 > "$scratch/dry-run.log"
find "$prefix" -type f | sort > "$scratch/after-dry"
cmp "$scratch/before" "$scratch/after-dry"
if make install-linux PREFIX=/usr DRY_RUN=1 > "$scratch/guard.log" 2>&1; then
    echo 'System guard did not refuse /usr' >&2; exit 1
fi
if make install-linux PREFIX=/usr/local DRY_RUN=1 >> "$scratch/guard.log" 2>&1; then
    echo 'System guard did not refuse /usr/local' >&2; exit 1
fi
make install-linux PREFIX=/usr/local SUDO=1 DRY_RUN=1 > "$scratch/confirmed.log"
make install-linux PREFIX="$prefix"
find "$prefix" -type f | sort > "$scratch/installed"
printf '%s\n' 'Installed files (including unrelated sentinels):'
while IFS= read -r file; do printf '%s\n' "$file"; done < "$scratch/installed"
{
    printf '%s\n' "$prefix/bin/glance" "$prefix/bin/other-app" \
        "$prefix/share/applications/glance.desktop" "$prefix/share/applications/other.desktop" \
        "$prefix/share/glance/install-manifest"
    for size in 16 24 32 48 64 128 256; do
        printf '%s/share/icons/hicolor/%sx%s/apps/glance.png\n' "$prefix" "$size" "$size"
    done
} | sort > "$scratch/expected"
cmp "$scratch/expected" "$scratch/installed"
cmp target/release/glance "$prefix/bin/glance"
test -x "$prefix/bin/glance"
cmp resources/glance.desktop "$prefix/share/applications/glance.desktop"
for size in 16 24 32 48 64 128 256; do
    cmp "assets/icons/hicolor/${size}x${size}/apps/glance.png" \
        "$prefix/share/icons/hicolor/${size}x${size}/apps/glance.png"
done
cp "$prefix/share/applications/glance.desktop" "$scratch/desktop-original"
printf '# changed\n' >> "$prefix/share/applications/glance.desktop"
if make uninstall-linux PREFIX="$prefix" > "$scratch/modified.log" 2>&1; then
    echo 'Modified-file guard failed' >&2; exit 1
fi
test -f "$prefix/bin/glance"
cp "$scratch/desktop-original" "$prefix/share/applications/glance.desktop"
make uninstall-linux PREFIX="$prefix" DRY_RUN=1 > "$scratch/uninstall-dry.log"
test -f "$prefix/bin/glance"
make uninstall-linux PREFIX="$prefix"
find "$prefix" -type f | sort > "$scratch/after"
printf '%s\n' 'After uninstall (only unrelated sentinels):'
while IFS= read -r file; do printf '%s\n' "$file"; done < "$scratch/after"
cmp "$scratch/before" "$scratch/after"
cksum "$prefix/bin/other-app" "$prefix/share/applications/other.desktop" > "$scratch/sentinel-after"
cmp "$scratch/sentinel-checksums" "$scratch/sentinel-after"
make uninstall-linux PREFIX="$prefix" > "$scratch/missing.log" 2>&1
grep 'manifest missing' "$scratch/missing.log"
# An unowned existing destination must never be overwritten.
printf 'unowned\n' > "$prefix/bin/glance"
if make install-linux PREFIX="$prefix" > "$scratch/unowned.log" 2>&1; then
    echo 'Unowned-file guard failed' >&2; exit 1
fi
rm -f "$prefix/bin/glance"
# Redirected destinations must not allow writes outside the selected prefix.
mkdir -p "$scratch/redirect/share" "$scratch/outside"
ln -s "$scratch/outside" "$scratch/redirect/share/glance"
if make install-linux PREFIX="$scratch/redirect" > "$scratch/symlink.log" 2>&1; then
    echo 'Symlink-directory guard failed' >&2; exit 1
fi
find "$scratch/outside" -type f > "$scratch/outside-after"
test ! -s "$scratch/outside-after"
make install-linux PREFIX=/usr DESTDIR="$scratch/stage"
test -x "$scratch/stage/usr/bin/glance"
make uninstall-linux PREFIX=/usr DESTDIR="$scratch/stage"
find "$scratch/stage" -type f > "$scratch/staged-after"
test ! -s "$scratch/staged-after"
# Simulate cache-tool failures under a scratch HOME, without real Linux tools.
# The real release build above supplies the payload; this cargo stub avoids
# searching a different HOME for Rust tooling during this cache-only check.
mkdir -p "$scratch/tools" "$scratch/cache-home/.local/share/mime"
printf '%s\n' '#!/bin/sh' 'exit 0' > "$scratch/tools/cargo"
for tool in update-desktop-database update-mime-database; do
    printf '%s\n' '#!/bin/sh' 'exit 1' > "$scratch/tools/$tool"
done
chmod 755 "$scratch/tools/cargo" "$scratch/tools/update-desktop-database" "$scratch/tools/update-mime-database"
HOME="$scratch/cache-home" PATH="$scratch/tools:$PATH" make install-linux > "$scratch/cache-install.log" 2>&1
grep 'Warning: desktop cache refresh failed' "$scratch/cache-install.log"
grep 'Warning: MIME cache refresh failed' "$scratch/cache-install.log"
HOME="$scratch/cache-home" PATH="$scratch/tools:$PATH" make uninstall-linux > "$scratch/cache-uninstall.log" 2>&1
test ! -f "$scratch/cache-home/.local/bin/glance"
echo "PASS: exact installed layout, dry-run, ownership and modification guards, sentinels, DESTDIR, nonfatal cache failures; logs in $scratch"
