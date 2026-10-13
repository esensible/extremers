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
| `otadata` | data/ota | 0xF000 | 8 KiB | which app slot to boot (ota_0 for now) |
| `phy_init` | data/phy | 0x11000 | 4 KiB | ESP-IDF standard; unused |
| `ota_0` | app | 0x20000 | 1.875 MiB | the app (about 1.65 MiB today) |
| `ota_1` | app | 0x200000 | 1.875 MiB | the other slot: the data slot (below); later an update |
| `settings` | data/nvs | 0x3E0000 | 128 KiB | key/value settings: WiFi SSID + password |

With `otadata` blank and no factory partition the bootloader boots `ota_0`,
which is where probe-rs flashes, and writes `ota_seq` 1 (= `ota_0`) into
`otadata`; the app logs `storage: otadata selects "Ota0" (ota_seq 0x1 /
0xffffffff)`. `provision` blanks `otadata` first: on a board flashed with
the earlier layout, its second sector held the old app image's header,
which esp-bootloader-esp-idf rejects (`storage: cannot read otadata`).

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
the data slot`), then what the store and the track hold. A firmware update
over the air (later) will write the new image over the data slot, so the
bundles are installed again and the track starts afresh afterwards (the
owner's choice); there is no rollback to the previous firmware.

Once OTA exists, mind that probe-rs always flashes `ota_0`: if `otadata`
then selects `ota_1`, the bootloader keeps booting `ota_1`. (`ota_seq` 1
survived three app flashes here, so a flash does not seem to clear
`otadata`.)

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

0x200000 is `ota_1`, the inactive slot while `ota_0` runs, which is where
probe-rs flashes the app. The app logs the data slot at boot (`storage:
running app slot ota_0; inactive slot ota_1 at 0x200000 …`), and `dump`
prints the exact command (`dump: bundles region at flash 0x200000 …`). The
image is exactly the region (512 KiB, whole sectors), so the GPS tracks after
it are untouched; it replaces every bundle there. At boot the app logs each
bundle it finds (`store: sectors 0+25: committed, seq 1, id 0x8ebbc298:
race v1, 101504 B`) and the free space; with one app and no boot package
kept, that app starts. An erased region gives Choose alone.

## WiFi credentials

At boot the app reads the SSID and WPA2 password from `settings`. If none are
stored, or they are invalid (SSID 1..=32 bytes, password 8..=63), it uses
`WIFI_SSID`/`WIFI_PASSWORD` from `common/src/config.rs`. The log says which
(`Wifi: stored credentials, ssid …` or `Wifi: no valid stored credentials,
compiled-in default ssid …`); the password itself is never logged.

The app writes only BLE bonds there: one when a watch pairs during the
pairing window, and it deletes them all on "Forget all watches" (Kindle:
Settings; see `../BLE.md` "Security"). To store credentials, flash the
`provision` tool, then the app again:

    EXTREMERS_SSID=nacra EXTREMERS_PSK='a passphrase' cargo run --release --bin provision
    cargo run --release

`provision` erases the settings partition (and blanks `otadata`, when
running from `ota_0`), writes the credentials, reads them back and logs the
result. The values come only from those environment
variables at build time: they are never in the source. Give them to the
same `cargo run` that flashes (cargo rebuilds when they change, so a bare
`cargo run --bin provision` would rebuild without them). Both or neither:
out-of-range values fail the build, and with neither set `provision` only
erases (a reset to the compiled-in defaults). Erasing deletes the BLE bonds
too: every watch must then forget `nacra` and pair again.

To see what is stored without changing it:

    cargo run --release --bin dump

It logs the stored SSID and the password's length, and the BLE bonds: how
many, and each one's address (never a key); the boot package kept; then
the data slot:

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
`../BLE.md` "The devtool" has the rest (stale keys on the computer, a Mac
that stops finding the box).

The firmware builds against a patched trouble-host clone at
`/Users/sensible-1/src/trouble` (root `Cargo.toml`, `[patch.crates-io]`); a
dev container needs it at that path, or the patch overridden on the command
line (`cargo --config 'patch.crates-io.trouble-host.path="…/host"' --config
'patch.crates-io.trouble-host-macros.path="…/host-macros"' …`).
