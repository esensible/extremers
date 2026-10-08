#!/bin/sh
# Typecheck the generated Swift with any Swift 6 toolchain (Linux is fine):
# the core NodeProps.swift as module SolidWatch, then an app fragment's file
# against it.
#   test/swift-typecheck.sh <NodeProps.swift> [<Fragment.swift> ...]
set -eu
core=${1:?NodeProps.swift}
shift
out=$(mktemp -d)
swiftc -swift-version 6 -parse-as-library -emit-module -module-name SolidWatch -emit-module-path "$out/SolidWatch.swiftmodule" "$core"
echo "core ok: $core"
for f in "$@"; do
  swiftc -swift-version 6 -typecheck -I "$out" "$f"
  echo "fragment ok: $f"
done
