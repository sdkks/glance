#!/bin/sh
# Validate the opt-in hook and print activation; never change Git configuration.
set -eu
cd "$(git rev-parse --show-toplevel)"

if [ ! -x .githooks/pre-commit ] || [ ! -f Makefile ]; then
    printf '%s\n' 'Missing executable .githooks/pre-commit or Makefile.' >&2
    exit 1
fi
sh -n .githooks/pre-commit

if configured=$(git config --get core.hooksPath); then
    if [ "$configured" != .githooks ]; then
        printf '%s\n' "Refusing conflicting core.hooksPath: $configured" >&2
        exit 1
    fi
    printf '%s\n' 'The tracked hooks path is already active.'
else
    status=$?
    if [ "$status" -ne 1 ]; then
        exit "$status"
    fi
    existing=$(git rev-parse --git-path hooks/pre-commit)
    if [ -e "$existing" ]; then
        printf '%s\n' 'Refusing to replace an existing pre-commit hook. Review it manually first.' >&2
        exit 1
    fi
fi

printf '%s\n' 'Validation passed. To activate from the repository root, run:' 'git config --local core.hooksPath .githooks'
