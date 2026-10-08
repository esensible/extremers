# Extremers for Apple Watch

A standalone watchOS app: a second client of the race computer, over BLE.
It shows the same state as the Kindle UI and has the same buttons (line
pins, start a sequence, bump or sync the countdown, finish). The device is
the only source of truth: the watch sends events and renders what the
device notifies; it never changes the race state itself.

The protocol is [`../BLE.md`](../BLE.md). `Extremers/Model/Protocol.swift`
is its Swift side and `ExtremersTests/ProtocolTests.swift` checks BLE.md's
test vectors byte for byte.

## Build

Needs a Mac with Xcode 16 or later (watchOS 10 SDK or later; the code uses
Swift 5 language mode and `@preconcurrency` conformances, which need the
Swift 6 compiler) and [XcodeGen](https://github.com/yonaskolb/XcodeGen).
The Xcode project is generated from `project.yml` and is not checked in.

```sh
brew install xcodegen
cd watch-app
xcodegen generate
open Extremers.xcodeproj
```

Re-run `xcodegen generate` after adding or removing files, or after
editing `project.yml`. It rewrites the project, so any change made in
Xcode's project editor (signing team included) is lost: put lasting
settings in `project.yml`.

Unit tests (protocol, formatting, haptic cues) run on a watch simulator:
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
  range, then the screen for whatever the device is doing (choose mode,
  race, tune).
- On the first connection it asks for Health permission to save workouts:
  allow it. The app then runs a sailing workout (green running-figure
  icon at the top of the watch face) for as long as it is connected,
  because watchOS suspends an app, and its Bluetooth link, when the wrist
  drops, unless a workout is running. Nothing is saved to Health.
- "Quit" on the choose-mode screen ends the workout and drops the link.
  Swiping the app away in the app switcher does the same.

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
  Model/Protocol.swift       UUIDs, DeviceState decode, DeviceEvent encode (pure Swift)
  Model/Commands.swift       buttons -> events, confirm requests (pure Swift)
  Model/Formatting.swift     mm:ss, knots, signed values, haptic cue times (pure Swift)
  Model/RaceComputer.swift   CoreBluetooth central: scan, connect, subscribe, reconnect
  Model/WorkoutKeeper.swift  HKWorkoutSession that keeps the app alive
  Model/Haptics.swift        countdown haptics from the anchored start
  Views/                     one file per screen, plus shared pieces
ExtremersTests/ProtocolTests.swift
```
