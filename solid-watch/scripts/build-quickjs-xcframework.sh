#!/bin/sh
# Builds Frameworks/CQuickJS.xcframework: the vendored quickjs-ng
# (quickjs-ng/, see VENDORED.txt) as an optimised static library for every
# platform Package.swift supports, with its header and a module map.
#
# Why a prebuilt binary: the interpreter must always be optimised. Built
# from source by SwiftPM it follows the app's configuration, and at -O0
# QuickJS's interpreter frames are several times larger, so an ordinary UI
# overflows the engine's stack limit in a Debug build. SwiftPM has no safe
# per-target optimisation setting (unsafeFlags would stop the package being
# used as a versioned dependency), so the library is built here, -O2,
# and the package links it as a binaryTarget.
#
# Needs Xcode's command line tools (xcrun, clang, libtool, lipo,
# xcodebuild). Run from the package root after updating quickjs-ng/:
#   sh scripts/build-quickjs-xcframework.sh
set -eu
cd "$(dirname "$0")/.."
# reproducible archives: no member timestamps
export ZERO_AR_DATE=1
SRC=quickjs-ng
OUT=Frameworks/CQuickJS.xcframework
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
    xcrun --sdk "$sdk" clang -target "$triple" $CFLAGS -I "$SRC" -I "$SRC/include" -c "$SRC/$f" -o "$dir/${f%.c}.o"
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
cp "$SRC/include/quickjs.h" "$HDR/"
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
echo "built $OUT"
