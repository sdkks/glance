#!/bin/sh
# Prepare an absent/stale stage; run from the repository root like bundle-avif.
set -eu
stage=$1
shift
refuse() {
    printf 'Refusing to refresh AVIF stage: %s; %s. Remove/refresh this path manually.\n' "$stage" "$1" >&2
    exit 1
}
if [ -d "$stage" ] || [ -e "$stage" ] || [ -L "$stage" ]; then
    binary="$stage/Contents/MacOS/glance"
    [ -f "$binary" ] || refuse 'missing Contents/MacOS/glance marker'
    # Keep this exact input set/comparison in sync with the fail-closed
    # backstop in scripts/package-macos.sh (covered by freshness tests).
    newer_inputs=$(find src Cargo.toml Cargo.lock build.rs \
        assets/icons/glance.iconset/icon_256x256.png -newer "$binary" -print)
    [ -n "$newer_inputs" ] || exit 0
    # Reject traversal and every symlink component, not only the final stage:
    # build/link/stage must never redirect deletion outside the repository.
    case "$stage" in
        */) refuse 'trailing slash path component' ;;
        build/*) ;;
        *) refuse 'automatic refresh requires a relative child of build/' ;;
    esac
    rest=$stage
    prefix=
    while [ -n "$rest" ]; do
        component=${rest%%/*}
        case "$component" in ''|.|..) refuse 'empty or traversal path component' ;; esac
        prefix=${prefix:+"$prefix/"}$component
        [ ! -L "$prefix" ] || refuse 'symlink path component'
        case "$rest" in */*) rest=${rest#*/} ;; *) rest= ;; esac
    done
    [ -d "$stage" ] || refuse 'not a stage directory'
    printf 'Refreshing stale AVIF stage: %s (newer executable inputs).\n' "$stage"
    rm -rf -- "$stage"
fi
MACOSX_DEPLOYMENT_TARGET=11.0 "$@" bundle-avif "AVIF_STAGE=$stage"
