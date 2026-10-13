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
| `ota_0` | app | 0x20000 | 1.875 MiB | the app (about 0.9 MiB today) |
| `ota_1` | app | 0x200000 | 1.875 MiB | the other slot: bundles (512 KiB at +0) and GPS tracks (640 KiB at +0x80000); later an update |
| `settings` | data/nvs | 0x3E0000 | 128 KiB | key/value settings: WiFi SSID + password |

With `otadata` blank and no factory partition the bootloader boots `ota_0`,
which is where probe-rs flashes, and writes `ota_seq` 1 (= `ota_0`) into
`otadata`; the app logs `storage: otadata selects "Ota0" (ota_seq 0x1 /
0xffffffff)`. `provision` blanks `otadata` first: on a board flashed with
the earlier layout, its second sector held the old app image's header,
which esp-bootloader-esp-idf rejects (`storage: cannot read otadata`). The slot that is not running holds GPS
tracks (not implemented yet): written while sailing, downloaded over BLE,
and erased before an OTA update writes new firmware into it. An update
therefore needs the tracks downloaded first, and there is no rollback to
the previous firmware once tracks have overwritten it. The app logs at
boot which slot it runs from and the inactive slot's size
(`storage::track_slot`).

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
store: the first 512 KiB of the app slot that is *not* running
(`common/src/store.rs`, `common/src/config.rs` `DATA_BUNDLES_*`). Until they
can be installed over BLE, write the whole region from an image built from
the repo's packages (from the repo root, in the dev container):

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
many, and each one's address (never a key); the boot package kept; the GPS
track's ring; and the bundle store's extents (state, seq, id, name and
version, length, sectors). Flash the app afterwards.

The firmware builds against a patched trouble-host clone at
`/Users/sensible-1/src/trouble` (root `Cargo.toml`, `[patch.crates-io]`); a
dev container needs it at that path, or the patch overridden on the command
line (`cargo --config 'patch.crates-io.trouble-host.path="…/host"' --config
'patch.crates-io.trouble-host-macros.path="…/host-macros"' …`).
