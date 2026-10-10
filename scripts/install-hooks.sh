#!/bin/sh
# Opt-in activation/removal of the tracked hooks; never copy hook files.
set -eu
cd "$(git rev-parse --show-toplevel)"

refuse() {
    printf '%s\n' "$1" 'Inspect configuration with: git config --show-origin --get-all core.hooksPath' >&2
    exit 1
}

case "${1:-install}" in
    install)
        if configured=$(git config --get core.hooksPath); then
            :
        else
            status=$?
            [ "$status" -eq 1 ] || exit "$status"
            configured=
        fi
        if [ "$configured" = .githooks ]; then
            printf '%s\n' 'Tracked hooks already active: pre-commit runs make precommit.'
            exit 0
        fi
        [ -z "$configured" ] || refuse "Refusing conflicting core.hooksPath: $configured"
        if [ ! -f .githooks/pre-commit ] || [ ! -x .githooks/pre-commit ] || [ ! -f Makefile ]; then
            printf '%s\n' 'Missing executable .githooks/pre-commit or Makefile; restore the tracked files and executable permission before retrying.' >&2
            exit 1
        fi
        sh -n .githooks/pre-commit
        git config --local core.hooksPath .githooks
        printf '%s\n' 'Tracked hooks activated: pre-commit runs make precommit.'
        ;;
    uninstall)
        # A sentinel preserves trailing empty values through command substitution.
        if configured=$(git config --local --get-all core.hooksPath && printf '.'); then
            configured=${configured%.}
            entry_count=$(printf '%s' "$configured" | awk 'END { print NR }')
            configured=${configured%?}
        else
            status=$?
            [ "$status" -eq 1 ] || exit "$status"
            configured=
            entry_count=0
        fi
        [ "$entry_count" -eq 1 ] && [ "$configured" = .githooks ] || refuse "Refusing to clear local core.hooksPath: found $entry_count local entr$( [ "$entry_count" -eq 1 ] && printf y || printf ies ); expected exactly one .githooks value. Inspect with: git config --local --show-origin --get-all core.hooksPath"
        git config --local --unset-all core.hooksPath
        printf '%s\n' 'Local tracked-hook activation removed; inherited hooksPath settings, if any, still apply.'
        ;;
    *)
        printf '%s\n' 'Usage: scripts/install-hooks.sh [install|uninstall]' >&2
        exit 1
        ;;
esac
