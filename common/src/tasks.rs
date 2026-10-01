//! Bodies of the tasks every target runs.
//!
//! `#[embassy_executor::task]` functions can't be generic, so each target
//! wraps these in a few lines of its own.

use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use edge_http::io::server::Server;
use edge_nal::TcpBind;
use embassy_time::{Duration, Timer};

use extreme_traits::RawEngine;

use crate::{
    config::{MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE},
    http::HttpHandler,
    nmea::{self, RingBuffer},
    runtime::EngineRuntime,
};

/// Serves the web UI and websocket on `port`, forever.
pub async fn serve_http<B, E>(bind: &B, port: u16, runtime: &EngineRuntime<E>) -> !
where
    B: TcpBind,
    E: RawEngine,
{
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
    let mut server: Server<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, 64> = Server::new();

    loop {
        match bind.bind(address).await {
            Ok(acceptor) => {
                log::info!("http: listening on port {}", port);
                if let Err(e) = server.run(None, acceptor, HttpHandler::new(runtime)).await {
                    log::error!("http: server error: {:?}", e);
                }
            }
            Err(e) => log::error!("http: cannot bind port {}: {:?}", port, e),
        }
        Timer::after(Duration::from_secs(1)).await;
    }
}

/// Feeds NMEA from `reader` to the runtime, forever.
pub async fn read_gps<R, E>(reader: R, runtime: &EngineRuntime<E>) -> !
where
    R: nmea::AsyncReader,
    E: RawEngine,
{
    let mut ring_buffer = RingBuffer::<R, 32>::new(reader);
    loop {
        let update = nmea::next_update(&mut ring_buffer).await;
        log::debug!("gps: {:?}", update);
        runtime.gps_update(update).await;
    }
}
