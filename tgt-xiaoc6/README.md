# tgt-xiaoc6

Firmware for the Seeed XIAO ESP32-C6 (4 MB flash). Logs are defmt over RTT,
read with probe-rs.

## Flashing

Run cargo from this directory: the runner in `.cargo/config.toml` is

    probe-rs run --chip esp32c6 --idf-partition-table partitions.csv

so `cargo run --release` flashes the bootloader, **this crate's partition
table** and the app, then prints the log. Flashing by hand must pass the same
`--idf-partition-table partitions.csv`; without it probe-rs writes its stock
table, the firmware logs `storage: no 'settings' partition` and runs on the
compiled-in WiFi defaults. Never pass `--chip-erase`: it wipes the settings.

## Partitions

| Name | Type | Offset | Size | Holds |
|---|---|---|---|---|
| `nvs` | data/nvs | 0x9000 | 24 KiB | ESP-IDF standard; unused |
| `phy_init` | data/phy | 0xF000 | 4 KiB | ESP-IDF standard; unused |
| `factory` | app | 0x10000 | 1.875 MiB | the app (about 0.9 MiB today) |
| `settings` | data/nvs | 0x1F0000 | 64 KiB | key/value settings: WiFi SSID + password |
| `tracks` | data/undefined | 0x200000 | 2 MiB | reserved for GPS tracks; unused |

A normal flash never erases `settings` or `tracks`: probe-rs erases only
around what it writes -- everything below 0x10000 (so the stock `nvs` does
not survive a flash, as hilux/wireless-can found) and the app image's
sectors -- the image is confined to `factory` (probe-rs refuses one that
does not fit), and `factory` ends where `settings` begins. The firmware finds both by label in the partition
table, so changing the layout means editing `partitions.csv` only. Details
in `partitions.csv` and `src/storage.rs`.

## WiFi credentials

At boot the app reads the SSID and WPA2 password from `settings`. If none are
stored, or they are invalid (SSID 1..=32 bytes, password 8..=63), it uses
`WIFI_SSID`/`WIFI_PASSWORD` from `common/src/config.rs`. The log says which
(`Wifi: stored credentials, ssid …` or `Wifi: no valid stored credentials,
compiled-in default ssid …`); the password itself is never logged.

The app never writes settings. To store credentials, flash the `provision`
tool, then the app again:

    EXTREMERS_SSID=nacra EXTREMERS_PSK='a passphrase' cargo run --release --bin provision
    cargo run --release

`provision` erases the settings partition, writes the credentials, reads
them back and logs the result. The values come only from those environment
variables at build time: they are never in the source. Give them to the
same `cargo run` that flashes (cargo rebuilds when they change, so a bare
`cargo run --bin provision` would rebuild without them). Both or neither:
out-of-range values fail the build, and with neither set `provision` only
erases (a reset to the compiled-in defaults).

To see what is stored without changing it:

    cargo run --release --bin dump

It logs the stored SSID and the password's length. Flash the app afterwards.
