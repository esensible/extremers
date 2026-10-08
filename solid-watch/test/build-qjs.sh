#!/bin/sh
# Build the quickjs-ng `qjs` CLI the tests run bundles in, without cmake: the
# same sources and defines as quickjs-ng's CMakeLists.txt (qjs_exe + the qjs
# library + quickjs-libc), Release flags. Use the quickjs-ng version the Swift
# package pins (v0.17.0), e.g. its submodule:
#   test/build-qjs.sh <quickjs-ng checkout> <output binary>
#   sh test/build-qjs.sh "$PWD/quickjs-ng" /tmp/qjs
set -eu
src=${1:?quickjs-ng checkout}
out=${2:?output path}
cd "$src"
cc -O2 -DNDEBUG -D_GNU_SOURCE -funsigned-char -w \
  -o "$out" \
  gen/repl.c gen/standalone.c qjs.c \
  quickjs-libc.c dtoa.c libregexp.c libunicode.c quickjs.c \
  -lm -ldl -lpthread
"$out" -e 'print("qjs " + (globalThis.navigator ? navigator.userAgent : ""))' || true
echo "built $out from $(git -C "$src" describe --tags --always 2>/dev/null || echo unknown)"
