# tgt-xiaoc6

Firmware for the Seeed XIAO ESP32-C6 (4 MB flash). Logs are defmt over RTT,
read with probe-rs.

## Flashing

Run cargo from this directory: the runner in `.cargo/config.toml` is

    probe-rs run --chip esp32c6 --idf-partition-table partitions.csv --idf-target-app-partition ota_0

so `cargo run --release` flashes the bootloader, **this crate's partition
table** and the app (into `ota_0`), then prints the log. Flashing by hand
must pass the same flags; without the table probe-rs writes its stock one,
the firmware logs `storage: no 'settings' partition` and runs on the
compiled-in WiFi defaults. Never pass `--chip-erase`: it wipes the settings.
With the probe on a `probe-rs serve` host, set `PROBE_RS_REMOTE_HOST` and
`PROBE_RS_REMOTE_TOKEN` and `cargo run` goes through it.

## Partitions

| Name | Type | Offset | Size | Holds |
|---|---|---|---|---|
| `nvs` | data/nvs | 0x9000 | 24 KiB | ESP-IDF standard; unused |
| `otadata` | data/ota | 0xF000 | 8 KiB | which app slot to boot |
| `phy_init` | data/phy | 0x11000 | 4 KiB | ESP-IDF standard; unused |
| `ota_0` | app | 0x20000 | 1.875 MiB | an app slot: the app (about 1.69 MiB today), or the data slot |
| `ota_1` | app | 0x200000 | 1.875 MiB | the other app slot: the data slot (below), or the app after a firmware update |
| `settings` | data/nvs | 0x3E0000 | 128 KiB | key/value settings: WiFi SSID + password, BLE bonds, the boot package, an update on trial |

With `otadata` blank and no factory partition the bootloader boots `ota_0`,
which is where probe-rs flashes, and writes `ota_seq` 1 (= `ota_0`) into
`otadata`; the app logs `storage: otadata selects "Ota0" (ota_seq 0x1 /
0xffffffff)`. Each firmware update moves the app to the other slot
(`ota_seq` one higher, in the other sector). `provision` blanks `otadata`
first: on a board flashed with the earlier layout, its second sector held
the old app image's header, which esp-bootloader-esp-idf rejects
(`storage: cannot read otadata`).

**The data slot.** The app slot that is not running is the *data slot*
(`src/storage.rs` `DataSlot`, `common/src/config.rs` `DATA_*`):

| From the slot's start | Size | Holds |
|---|---|---|
| +0x00000 | 512 KiB | the bundle store (`common/src/store.rs`): every installed app, written by an install over BLE (`../BLE.md` "Bundles") or a dev image (below) |
| +0x80000 | 640 KiB | the GPS track (`common/src/track.rs`): a ring of 32-byte fixes, 20,480 of them (5.7 h at 1 Hz), appended on every fix |
| +0x120000 | to the end | spare |

Both regions are read in place through the flash MMU (`src/xip.rs`). The
app logs at boot which slot it runs from and where the data slot is
(`storage: running app slot ota_0; inactive slot ota_1 at 0x200000, …:
the data slot`), then what the store and the track hold.

**A firmware update** over BLE (`../BLE.md` "Firmware update"; the studio
does it) writes the new image over the data slot, then boots it: the slots
swap roles, and the slot the old firmware ran from is the data slot from
then on. The bundles and the track in the data slot are lost (the owner's
choice): the studio saves the bundles off the box first and installs them
again afterwards; the track starts afresh. The new firmware runs **on
trial** for its first 20 s (`firmware: on trial after an update from ota_N:
boot 1 of 3; …`): the data slot then still holds the old firmware, its way
back, so nothing writes it (the track is in RAM, bundle installs and
deletes are refused with `Busy`), a panic resets instead of halting, and
an RTC watchdog (10 s, fed by the LED loop) resets a hang. Each boot on
trial is counted in `settings` (`KEY_OTA`); the third makes `otadata`
select the old slot again and resets. After 20 s it confirms itself
(`firmware: confirmed after 20 s; the data slot is writable`): the record
goes, `otadata` marks the slot `Valid`, the track opens in flash, bundles
can be installed. If the bootloader refuses the new image (it checks an
image before booting it) it boots the old slot, and the old firmware drops
the record (and heals `otadata`, below).

**`otadata` and probe-rs.** probe-rs always flashes `ota_0`, and leaves
`otadata` as it is, or nearly: `probe-rs run` was seen twice to erase its
first sector (`ota_seq 0xffffffff / 0x2` afterwards). So:

- Running `ota_0`, with `ota_1` holding bundles (no image at its start):
  if `otadata` ends up selecting `ota_1`, the bootloader finds no image
  there and falls back to `ota_0`. The firmware then heals `otadata` at
  boot: `storage: otadata selected ota_1 but ota_0 runs (the bootloader
  fell back): now selects ota_0` (`src/storage.rs` `select_running`; it
  does this whenever no update is on trial). Nothing to do.
- Running `ota_1` (after an odd number of updates): `otadata` selects
  `ota_1`, which holds a good image, so after a flash into `ota_0` the box
  keeps booting the old firmware in `ota_1`, which takes `ota_0` for its
  data slot and erases a sector of the image just flashed (its track
  starts there, at +0x80000). `dump` warns (`dump: otadata selects ota_1
  …`). **Blank `otadata` first** (the settings stay as they are), then
  flash:

      cargo run -p extreme-image -- otadata-blank      # repo root: target/otadata-blank.bin, 8 KiB of 0xFF
      probe-rs download --chip esp32c6 --binary-format bin --base-address 0xF000 target/otadata-blank.bin

  Blank selects no slot, so the bootloader boots `ota_0`. The bundles were
  in `ota_0`, the slot the flash overwrites: install them again (or bring
  the box back to `ota_0` with a firmware update from the studio instead,
  which keeps them).

A normal flash never erases `settings` or `ota_1`: probe-rs erases only
around what it writes -- the region of the bootloader and table (on
hilux/wireless-can it wiped the stock `nvs` at 0x9000) and the app image's
sectors in `ota_0` -- and refuses an image bigger than the slot. The
firmware finds partitions by label or type in the table, so changing the
layout means editing `partitions.csv` only. Details in `partitions.csv` and
`src/storage.rs`.

## Installing bundles for development

The firmware holds only Choose. Apps (Race, …) are bundles in the bundle
store, the data slot's first 512 KiB. They are installed over BLE from a
browser (`../BLE.md` "Bundles"; the first connection must pair, see
"Pairing without a Kindle" below). Without a browser, write the whole
region from an image built from the repo's packages (from the repo root,
in the dev container):

    cargo run -p extreme-pack -- --bundle --store-image target/packages/bundles.img

then flash it at the inactive slot's start, after the app (`cargo run
--release` here):

    probe-rs download --chip esp32c6 --binary-format bin --base-address 0x200000 target/packages/bundles.img

0x200000 is `ota_1`, the data slot while `ota_0` runs, which is where
probe-rs flashes the app. After a firmware update the box may run `ota_1`
instead, and its data slot is then `ota_0` at 0x20000: the address depends
on the running slot, so take it from the app's boot log (`storage: running
app slot ota_0; inactive slot ota_1 at 0x200000 …`) or from `dump`, which
prints the exact command (`dump: bundles region at flash 0x200000 …`). The
image is exactly the region (512 KiB, whole sectors), so the GPS tracks after
it are untouched; it replaces every bundle there. At boot the app logs each
bundle it finds (`store: sectors 0+25: committed, seq 1, id 0x8ebbc298:
race v1, 101504 B`) and the free space; with one app and no boot package
kept, that app starts. An erased region gives Choose alone.

A bundle that isn't one of the repo's packages (one authored in the studio
and saved as a `.bundle` file) goes in the same way when BLE isn't
available: build the image from bundle files instead, with `--from` once
per file:

    cargo run -p extreme-pack -- --store-image target/packages/bundles.img --from my-app.bundle --from race.bundle

No package is built then: the image holds exactly those files, in that
order (seq 1, 2, …). Each must be an app bundle (not Choose) whose manifest
reads and matches its files; anything else is refused, naming the file.
It prints a line per bundle (name, id, version, length) and the space left
in the region, then flash it as above:

    probe-rs download --chip esp32c6 --binary-format bin --base-address 0x200000 target/packages/bundles.img

As before, it replaces every bundle in the region: to keep the repo's
apps, list their bundles (`--bundle` writes `target/packages/<name>.bundle`)
too.

## Firmware images

A firmware update takes the app as the bootloader does, ESP-IDF's app
image with its appended SHA-256 digest (`../BLE.md` "Firmware update").
Make it from the release ELF (from the repo root, in the dev container,
after `cargo build --release` here):

    cargo run -p extreme-image -- target/riscv32imac-unknown-none-elf/release/xiaoc6

It writes `target/xiaoc6.bin` (`-o <path>` for another) with espflash's
library, as probe-rs makes the image it flashes (4 MB, DIO, 40 MHz, this
partition table, `ota_0`), checks it (`extreme_protocol::firmware::check`
and the digest), and prints its length, digest and version. The digest is
the one the app logs at boot (`firmware: running ota_N, … B, sha256 …,
version …`) and BLE's `Firmware` reports. Drop the `.bin` on the studio's
Device panel (or `api.device.updateFirmware({ base64 })`). Raise
`version` in `Cargo.toml` for an image meant for a box.

To test the way back, build an image that never confirms itself:
`--features ota-crash-test` panics 5 s after boot, `--features
ota-hang-test` hangs then (the watchdog resets it). Updated to one, the box
boots it three times and goes back to the firmware it came from (`firmware:
on trial, boot 3 of 3 without confirming: rolling back to ota_N`). Never
for a box in use.

## Before potting

Once the box is sealed only BLE reaches it. The firmware recovers on its
own from an update that fails to boot properly, as far as the bootloader
lets it:

- The bootloader flashed today (probe-rs's, ESP-IDF v5.5.1, default
  configuration) has **no rollback**: it ignores the slot's state. It does
  check an image before booting it and boots the other slot when the
  selected one fails, so an image damaged in flash is survived.
- The firmware itself catches an update that panics or hangs within 20 s
  of booting (the boot count, three boots, then back), but only once it
  has got as far as counting: a crash earlier in boot (`esp_hal::init`,
  the heap, the scheduler, the partition table) loops for good.
- It already writes what a rollback bootloader needs: `New` when it
  switches to an update, `Valid` when the update confirms itself.

So, before potting: build an ESP-IDF v5.5.x bootloader for the ESP32-C6
with `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y` (4 MB flash, DIO, 40 MHz,
as now) and flash it once, with the app, by adding `--idf-bootloader
<bootloader.bin>` to probe-rs's `--idf-*` flags. A `New` slot is then
booted once as `PendingVerify`; booted again without being made `Valid`
it is marked `Aborted` and the other slot boots, which catches the early
crash too (any reset during the trial then rolls back, power loss
included). Not done or tried yet. Then:

- provision the WiFi credentials, pair the watches and the computer that
  will update it;
- flash a release build without `dev-pairing`, `selftest` or `ota-*-test`;
- do an update and back from the studio on the finished box (once sealed
  it is the only way in), and check the boot log or `dump`: `otadata`
  selects the running slot, no update on trial, the bundles there.

## WiFi credentials

At boot the app reads the SSID and WPA2 password from `settings`. If none are
stored, or they are invalid (SSID 1..=32 bytes, password 8..=63), it uses
`WIFI_SSID`/`WIFI_PASSWORD` from `common/src/config.rs`. The log says which
(`Wifi: stored credentials, ssid …` or `Wifi: no valid stored credentials,
compiled-in default ssid …`); the password itself is never logged.

The app writes only BLE bonds there (one when a watch pairs during the
pairing window, and it deletes them all on "Forget all watches"; Kindle:
Settings; see `../BLE.md` "Security"), their GATT state, the boot package,
and a firmware update's record while it is on trial. To store credentials, flash the
`provision` tool, then the app again:

    EXTREMERS_SSID=nacra EXTREMERS_PSK='a passphrase' cargo run --release --bin provision
    cargo run --release

`provision` erases the settings partition (and blanks `otadata`, when
running from `ota_0`: anywhere else, blank `otadata` would switch the next
boot to `ota_0`; see "`otadata` and probe-rs" for that case), writes the
credentials, reads them back and logs the result. The values come only from those environment
variables at build time: they are never in the source. Give them to the
same `cargo run` that flashes (cargo rebuilds when they change, so a bare
`cargo run --bin provision` would rebuild without them). Both or neither:
out-of-range values fail the build, and with neither set `provision` only
erases (a reset to the compiled-in defaults). Erasing deletes the BLE bonds
too: every watch must then forget `nacra` and pair again.

To see what is stored without changing it:

    cargo run --release --bin dump

It logs the stored SSID and the password's length, and the BLE bonds: how
many, and each one's address (never a key); the boot package kept; a
firmware update on trial, if any, and the slot `otadata` selects (with a
warning when it is `ota_1`); then the data slot:

    dump: data slot 0x200000+0x1e0000: bundles 0x200000..0x280000, tracks 0x280000..0x320000, spare from 0x320000
    dump: track: Ring, head … (the next boot's run starts here), oldest …, … of 20480 records held, keep …
    dump: track: newest-0: t …, lat …, lon …, alt …
    dump: bundles region at flash 0x200000 (probe-rs download --binary-format bin --base-address 0x200000 bundles.img)
    dump: bundles: sectors 0+25: committed, seq 1, id 0x8ebbc298, 101504 B: race v1
    dump: bundles: 1 extents, 103 sectors without one

(the track's ring as the app's boot scan finds it, with its three newest
fixes; each bundle extent's state, sequence number, id, length, and its
manifest's name and version). It only reads. Flash the app afterwards.

## Pairing without a Kindle

New BLE bonds are made only while the pairing window is open, which the
Kindle's settings panel opens (`../BLE.md` "The pairing window"). For
development without a Kindle, build with `--features dev-pairing` (off by
default; never for a boat): the window opens once at boot, for 120 s
(`ble: dev-pairing: window open for 120 s`), so a browser can pair and
install bundles:

    cargo run --release --features dev-pairing

A reflash or reset opens it again. Anyone in range can pair meanwhile.
`../BLE.md` "The studio's client" and "Pairing without a Kindle" have the rest (stale keys on the computer, a Mac
that stops finding the box).

The firmware builds against a patched trouble-host clone at
`/Users/sensible-1/src/trouble` (root `Cargo.toml`, `[patch.crates-io]`); a
dev container needs it at that path, or the patch overridden on the command
line (`cargo --config 'patch.crates-io.trouble-host.path="…/host"' --config
'patch.crates-io.trouble-host-macros.path="…/host-macros"' …`).
