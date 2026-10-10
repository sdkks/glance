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
        if configured=$(git config --local --get core.hooksPath); then
            :
        else
            status=$?
            [ "$status" -eq 1 ] || exit "$status"
            configured=
        fi
        [ "$configured" = .githooks ] || refuse "Refusing to clear local core.hooksPath: ${configured:-unset/empty} (expected .githooks)."
        git config --local --unset-all core.hooksPath
        printf '%s\n' 'Local tracked-hook activation removed; inherited hooksPath settings, if any, still apply.'
        ;;
    *)
        printf '%s\n' 'Usage: scripts/install-hooks.sh [install|uninstall]' >&2
        exit 1
        ;;
esac
