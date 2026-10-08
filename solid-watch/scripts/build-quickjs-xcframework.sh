#!/bin/sh
# Builds Frameworks/CQuickJS.xcframework: quickjs-ng (the quickjs-ng/
# submodule, v0.17.0) as an optimised static library for every platform
# Package.swift supports, with its header and a module map.
#
# Why a prebuilt binary: the interpreter must always be optimised. Built
# from source by SwiftPM it follows the app's configuration, and at -O0
# QuickJS's interpreter frames are several times larger, so an ordinary UI
# overflows the engine's stack limit in a Debug build. SwiftPM has no safe
# per-target optimisation setting (unsafeFlags would stop the package being
# used as a versioned dependency), so the library is built here, -O2,
# and the package links it as a binaryTarget.
#
# The framework is a build output (Frameworks/ is ignored): run this once
# after cloning, and again after updating the submodule. The build is
# reproducible (same sources, same Xcode: same bytes).
#
# Needs Xcode's command line tools (xcrun, clang, libtool, lipo,
# xcodebuild) and the submodule checked out:
#   git submodule update --init quickjs-ng     # in solid-watch/
#   sh scripts/build-quickjs-xcframework.sh
set -eu
cd "$(dirname "$0")/.."
SRC=quickjs-ng
OUT=Frameworks/CQuickJS.xcframework
if [ ! -f "$SRC/quickjs.c" ] || [ ! -f "$SRC/quickjs.h" ]; then
  echo "error: $SRC/ is not checked out. Run 'git submodule update --init quickjs-ng'" >&2
  echo "in solid-watch/ (or 'git submodule update --init' at the repository root)." >&2
  exit 1
fi
# reproducible archives: no member timestamps
export ZERO_AR_DATE=1
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The same defines the package used for the source target in release.
CFLAGS="-O2 -DNDEBUG -D_GNU_SOURCE -fno-common -w -std=gnu11"
FILES="quickjs.c libregexp.c libunicode.c dtoa.c"

# slice: <sdk> <target triple>
build() {
  sdk=$1 triple=$2
  dir="$WORK/obj/$triple"
  mkdir -p "$dir"
  for f in $FILES; do
    xcrun --sdk "$sdk" clang -target "$triple" $CFLAGS -I "$SRC" -c "$SRC/$f" -o "$dir/${f%.c}.o"
  done
  xcrun --sdk "$sdk" libtool -static -o "$dir/libCQuickJS.a" "$dir"/*.o
  echo "$dir/libCQuickJS.a"
}

# platform: <name> <sdk> <triple...>: one fat library per platform
platform() {
  name=$1 sdk=$2
  shift 2
  libs=""
  for t in "$@"; do libs="$libs $(build "$sdk" "$t")"; done
  mkdir -p "$WORK/lib/$name"
  xcrun lipo -create $libs -output "$WORK/lib/$name/libCQuickJS.a"
}

platform watchos watchos arm64_32-apple-watchos10.0 arm64-apple-watchos10.0
platform watchsim watchsimulator arm64-apple-watchos10.0-simulator x86_64-apple-watchos10.0-simulator
platform ios iphoneos arm64-apple-ios17.0
platform iossim iphonesimulator arm64-apple-ios17.0-simulator x86_64-apple-ios17.0-simulator
platform macos macosx arm64-apple-macos14.0 x86_64-apple-macos14.0

HDR="$WORK/include"
mkdir -p "$HDR"
cp "$SRC/quickjs.h" "$HDR/"
cat > "$HDR/module.modulemap" <<'EOF'
module CQuickJS {
    header "quickjs.h"
    export *
}
EOF

rm -rf "$OUT"
mkdir -p Frameworks
args=""
for p in watchos watchsim ios iossim macos; do
  args="$args -library $WORK/lib/$p/libCQuickJS.a -headers $HDR"
done
xcodebuild -create-xcframework $args -output "$OUT"
cp "$SRC/LICENSE" "$OUT/LICENSE"
echo "built $OUT from quickjs-ng $(git -C "$SRC" describe --tags --always 2>/dev/null || echo '(unknown version)')"
