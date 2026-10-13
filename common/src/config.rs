//! Build-time configuration shared by every target.

use core::net::Ipv4Addr;

/// Name of the WiFi access point the embedded targets create.
pub const WIFI_SSID: &str = "nacra";
/// WPA2 passphrase of the access point.
pub const WIFI_PASSWORD: &str = "password";
/// WiFi channel of the access point.
pub const WIFI_CHANNEL: u8 = 10;

/// Name the device advertises over BLE (BLE.md).
pub const BLE_NAME: &str = "nacra";
/// Concurrent BLE centrals: the watch plus a phone or a debugging tool.
pub const MAX_BLE_CONNECTIONS: usize = 2;
/// Bonded devices kept (BLE.md "Security"): a crew's watches and a phone.
/// When all are taken, a new pairing replaces the oldest bond.
pub const MAX_BLE_BONDS: usize = extreme_protocol::MAX_BLE_BONDS;
/// How long "Pair watch" on the Kindle's settings panel lets a new device
/// bond (BLE.md "Pairing a watch"). Bonding is refused at any other time;
/// bonded devices reconnect whenever they like.
pub const BLE_PAIRING_WINDOW_SECS: u64 = 120;

/// The device's own address on the access point network. It is also the
/// gateway and DNS server handed out by DHCP.
pub const AP_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 100);
/// Prefix length of the access point network.
pub const AP_PREFIX_LEN: u8 = 24;

/// The page DHCP advertises as the network's captive portal (option 114),
/// so a client that looks for one opens the race page by itself. Any name
/// resolves to [`AP_IP`] (the captive DNS server), so this one is only what
/// clients see. A dotted name so browsers look it up rather than search, in
/// a TLD browsers don't force to HTTPS.
pub const CAPTIVE_URL: &str = "http://nacra.race/";

/// Port the web UI is served on by the embedded targets.
pub const HTTP_PORT: u16 = 80;

/// Baud rate of the GPS UART.
pub const GPS_BAUD: u32 = 9600;
/// Fixes the RAM track keeps (`track::RamTrack`, 24 B each): the newest,
/// for the engines' `gps` on a host without a flash track (`tgt-std`, the
/// tests).
pub const TRACK_FIXES: usize = 128;

// The C6's data slot: the OTA app slot the running firmware was not booted
// from (tgt-xiaoc6/src/storage.rs, `DataSlot`). Offsets from the slot's
// start; both regions are whole 64 KiB MMU pages. A firmware update written
// into the slot wipes both, by choice: they are installed and recorded
// again afterwards.

/// Where the bundles region starts in the data slot.
pub const DATA_BUNDLES_OFFSET: u32 = 0;
/// The bundles region's size: 512 KiB, four or five bundles.
pub const DATA_BUNDLES_LEN: u32 = 0x8_0000;
/// Where the tracks region starts in the data slot, right after the bundles.
pub const DATA_TRACKS_OFFSET: u32 = DATA_BUNDLES_OFFSET + DATA_BUNDLES_LEN;
/// The tracks region's size: 640 KiB of 32-byte fixes (`track::FlashTrack`)
/// = 20,480, 5.7 h at 1 Hz.
pub const DATA_TRACKS_LEN: u32 = 0xA_0000;
/// The data slot's bytes in use; the rest of the slot is spare.
pub const DATA_SLOT_USED: u32 = DATA_TRACKS_OFFSET + DATA_TRACKS_LEN;
/// The most fixes an engine's `gps` holds: the tracks region in records.
/// Equal to `engine_profile::GPS_MAX_ITEMS`, the `maxItems` every engine is
/// compiled against (engine/PROFILE.md §2).
pub const GPS_MAX_ITEMS: u32 = DATA_TRACKS_LEN / crate::track::RECORD_LEN as u32;

const _: () = assert!(DATA_TRACKS_OFFSET % 0x1_0000 == 0 && DATA_SLOT_USED % 0x1_0000 == 0);

/// Maximum number of simultaneous HTTP connections, websockets included.
pub const MAX_WEB_SOCKETS: usize = 4;
/// Size of each TCP socket buffer and of the HTTP server's per-connection
/// buffer.
pub const SOCKET_BUFFER_SIZE: usize = 2048;
/// Largest websocket frame taken from a client. Client events are a few
/// bytes (postcard); a longer frame fails the connection.
pub const WS_MAX_INCOMING: usize = 512;

/// How long an HTTP connection may sit idle between requests before it is
/// closed to free its slot; browsers simply open a new one.
pub const HTTP_KEEPALIVE_TIMEOUT_MS: u32 = 5_000;
/// Longest any single socket operation (read, write, flush, close) may
/// take. A client that vanishes without closing its connection (a Kindle
/// going to sleep or out of range) never acknowledges data, and with no
/// limit a write, or the final close, would wait for it forever.
pub const SOCKET_IO_TIMEOUT_MS: u32 = 10_000;
/// How often a websocket is pinged and re-sent the current state. The
/// clients reconnect after 15 s without a message (`SILENCE_TIMEOUT_MS` in
/// each client-js), so keep this well below that.
pub const WS_HEARTBEAT_MS: u64 = 5_000;
/// A websocket client that has sent nothing, not even a pong, for this long
/// is taken to be gone and disconnected.
pub const WS_CLIENT_TIMEOUT_MS: u64 = 15_000;

// The websocket loop waits for the client to be readable for at most one
// heartbeat at a time; it must not trip the socket's own I/O timeout.
const _: () = assert!(WS_HEARTBEAT_MS < SOCKET_IO_TIMEOUT_MS as u64);
