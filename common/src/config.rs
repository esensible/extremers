//! Build-time configuration shared by every target.

use core::net::Ipv4Addr;

use extreme_traits::MAX_MESSAGE_SIZE;

/// Name of the WiFi access point the embedded targets create.
pub const WIFI_SSID: &str = "nacra";
/// WPA2 passphrase of the access point.
pub const WIFI_PASSWORD: &str = "password";
/// WiFi channel of the access point.
pub const WIFI_CHANNEL: u8 = 10;

/// The device's own address on the access point network. It is also the
/// gateway and DNS server handed out by DHCP.
pub const AP_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 100);
/// Prefix length of the access point network.
pub const AP_PREFIX_LEN: u8 = 24;

/// Port the web UI is served on by the embedded targets.
pub const HTTP_PORT: u16 = 80;

/// Baud rate of the GPS UART.
pub const GPS_BAUD: u32 = 9600;

/// Maximum number of simultaneous HTTP connections, websockets included.
pub const MAX_WEB_SOCKETS: usize = 4;
/// Size of each TCP socket buffer and of the HTTP server's per-connection
/// buffer.
pub const SOCKET_BUFFER_SIZE: usize = MAX_MESSAGE_SIZE * 4;

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
