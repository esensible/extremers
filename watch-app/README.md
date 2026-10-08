# Extremers for Apple Watch

A standalone watchOS app: a second client of the race computer, over BLE.
It is a shell. It owns the link (find the box, pair, reconnect, keep
running with the wrist down) and its own screens (connecting, pairing
problems, a box speaking another protocol). Each app on the box (a package:
Choose, Race, Tune) brings its own watch UI: a
[solid-watch](../solid-watch/README.md) bundle, structured like the
package's Kindle page, that the shell runs. The box is the only source of
truth. It says which package is active, and the watch renders what the box
notifies and sends events. It never changes anything itself.

The box's `state` value is a postcard `Envelope { timestamp, active,
ui_hash, state }`. The shell reads `active` and `ui_hash` to pick the
bundle (`Model/Protocol.swift`, `Model/Bundles.swift`), and hands every
envelope's bytes to the running bundle. The bundle decodes them, keeps the
box's clock offset, and sends `HostEvent` bytes, which the shell writes to
the `event` characteristic. A package this watch has no bundle for shows
"This app isn't on the watch" with an Exit (`SelectDefault`). The
envelope and event types are the generated host bindings,
[`protocol-swift`](../protocol-swift) (`tools/bindgen`; see the main
README, "Host protocol"); the bundles use the JS ones (`protocol-js/` and
each package's `protocol-js/`). `ExtremersTests/EnvelopeTests.swift`
checks the shell's reading of them against the Rust encoder's vectors
(`protocol-js/test/vectors.json`).

For now the bundles are compiled in. A build phase copies each package's
`watch-js/fixtures/bundle.js` (its committed build) and its sha256 into
`Bundles/<package name>/`. Delivery over BLE takes the place of
`CompiledBundles` later.

## Build

Needs a Mac with Xcode 16 or later (watchOS 10 SDK or later; the code uses
Swift 5 language mode and `@preconcurrency` conformances, which need the
Swift 6 compiler) and [XcodeGen](https://github.com/yonaskolb/XcodeGen).
The Xcode project is generated from `project.yml` and is not checked in.

```sh
sh solid-watch/scripts/build-quickjs-xcframework.sh   # once, and after updating quickjs-ng
brew install xcodegen
cd watch-app
xcodegen generate
open Extremers.xcodeproj
```

solid-watch links QuickJS as a prebuilt, always optimised XCFramework
(`solid-watch/Frameworks/`, a build output, not committed; see
`solid-watch/README.md`): build it once as above before the first Xcode
build.

Re-run `xcodegen generate` after adding or removing files, or after
editing `project.yml`. It rewrites the project, so any change made in
Xcode's project editor (signing team included) is lost: put lasting
settings in `project.yml`.

Unit tests (the envelope, bundle routing, link bookkeeping, pairing decisions, the paired box) run on a watch simulator:
pick a watchOS simulator and press Cmd-U. They need no device.

## Signing

1. Xcode > Settings > Accounts: add your Apple ID. A free account works
   (a "Personal Team").
2. Select the `Extremers` target > Signing & Capabilities, tick
   "Automatically manage signing", choose your team. To keep it across
   regenerations, set `DEVELOPMENT_TEAM` in `project.yml` to the team ID
   shown there instead.
3. The bundle ID `au.esensible.extremers.watch` is a placeholder; if Xcode
   says it is taken, change `PRODUCT_BUNDLE_IDENTIFIER` in `project.yml`
   (and the tests' one) to something of your own and regenerate.
4. HealthKit is in the entitlements (for the workout session). If your
   team cannot use HealthKit, Xcode says so here; see "No workout" below.

With a free account, apps expire **7 days** after install: rebuild and run
from Xcode again each week. Paid accounts get a year.

## Install on the watch

1. Pair the watch with an iPhone that is connected to the Mac (USB or the
   same Wi-Fi). On the watch: Settings > Privacy & Security > Developer
   Mode on (the option appears after Xcode first sees the watch; the watch
   restarts). The iPhone needs Developer Mode too.
2. In Xcode's toolbar pick the `Extremers` scheme and your watch as the
   run destination (it can take a few minutes to appear and to "prepare"
   the first time).
3. Run (Cmd-R). The first install is slow over the phone's link.
4. With a free account the first launch is refused until you trust the
   developer: on the iPhone, Settings > General > VPN & Device Management
   > your Apple ID > Trust (on recent watchOS the watch may ask instead).

## First run

- The watch asks for Bluetooth access: allow it, or the Connecting screen
  says what to do.
- It shows "Looking for nacra" until the race computer is powered and in
  range, then the active app's screen (Choose, Race, Tune).
- The race computer only talks to a watch paired with it (BLE.md,
  "Pairing a watch"). The first time: on the Kindle, Settings → **Pair
  watch**, then open the app; the watch asks to pair with `nacra`: tap
  **Pair**. Until then the app says "Not paired" and what to do, and tries
  again by itself every 10 s. After that it reconnects without asking.
- The app remembers that box ("Paired box 8E:96:10", the end of the
  Kindle's "This box") and ignores other boats' boxes. To use another box:
  Quit, **Forget this box**, Connect. That clears only the app's memory;
  the pairing itself is removed in the watch's Settings → Bluetooth.
- "Pairing lost" means the box no longer has this watch's pairing (Forget
  all watches, or the box was re-provisioned): forget `nacra` in the
  watch's Settings → Bluetooth, then pair again from the Kindle.
- On first launch it asks for Health permission to save workouts: allow
  it. The app then runs a sailing workout (green running-figure icon at
  the top of the watch face) from the moment it starts looking for the
  race computer until Quit, because watchOS suspends an app, and its
  Bluetooth link (or its search for one), when the wrist drops, unless a
  workout is running. Nothing is saved to Health.
- watchOS runs one workout at a time: starting a workout in another app
  ends this one. Extremers starts it again (at once if it is in front,
  else when you come back to it) and says "Workout restarted".
- The **⋯** button at the left edge of Choose (the other apps have the
  whole screen) shows the box and the link, and **Quit**, which ends the
  workout and drops the link. Swiping the app away in the app switcher
  does the same.

### The shell's log

The app keeps a rolling log, `Documents/shell.log` (1 MB, then moved to
`shell.log.1`). It records the link (Bluetooth state, connects with the
ATT MTU, drops), every envelope (active package, ui_hash, size, how it was
routed, the bundle's timer counts), bundle loads and failures, taps, every
event sent with its bytes and whether the box answered the write, haptics
and main-thread stalls. Copy it off the watch with:

```sh
xcrun devicectl device copy from --device <udid> --domain-type appDataContainer \
  --domain-identifier au.esensible.extremers.watch --source Documents/shell.log --destination shell.log
```

### No workout

If HealthKit is refused (or the team cannot sign it), everything works
while the screen is on; with the wrist down watchOS suspends the app, the
link drops, and the app reconnects (banner) when it is raised. The
countdown keeps running from the last state either way.

## Layout

```
project.yml                  XcodeGen spec: app + unit tests
Extremers/
  ExtremersApp.swift
  Model/Protocol.swift       UUIDs, package ids, Envelope decode, SelectDefault (pure Swift)
  Model/Bundles.swift        BundleSource, the compiled-in bundles, ShellRouter (pure Swift)
  Model/Shell.swift          runs the active package's bundle (SolidWatchSession)
  Model/LinkPolicy.swift     backoff, loop generations, waiter table, workout restart rule (pure Swift)
  Model/Security.swift       CoreBluetooth errors -> retry / pairing problem / notice (no CB objects)
  Model/Box.swift            the paired box: scan-response address, matching, UserDefaults (pure Swift)
  Model/RaceComputer.swift   CoreBluetooth central: scan, connect, secure (pair), reconnect
  Model/WorkoutKeeper.swift  HKWorkoutSession that keeps the app alive
  Views/                     the shell's screens and its menu
ExtremersTests/EnvelopeTests.swift
ExtremersTests/ShellRouterTests.swift
ExtremersTests/LinkPolicyTests.swift
ExtremersTests/SecurityTests.swift
ExtremersTests/BoxTests.swift
```

The apps' UIs live with their packages: `extreme-race/watch-js`,
`extreme-tune/watch-js`, `choose/watch-js` (Choose). Haptics are
theirs too. The Race bundle's countdown buzzes each minute, 30 s, 10 s,
the last five seconds and the gun through solid-watch's haptic node.
