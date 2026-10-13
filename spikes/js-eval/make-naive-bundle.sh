#!/bin/sh
# make-naive-bundle.sh <dest>
#
# Copies what a naive agent may have into <dest>: the validator (stages a
# and b work; c reports the tests missing), LANGUAGE.md and the four task
# folders. Leaves out tests/, reference/ and example/. <dest> must sit next
# to js-engines (go.mod points at ../js-engines).
set -e
[ $# -eq 1 ] || { echo "usage: $0 <dest>" >&2; exit 2; }
src=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$1"
for f in check go.mod go.sum LANGUAGE.md cmd lint schema running riding vendee ultra; do
	cp -R "$src/$f" "$1/"
done
# lint's own tests read the hidden references: not for naive agents
rm -f "$1/lint/lint_test.go"
echo "naive bundle in $1"
