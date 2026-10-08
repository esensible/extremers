//! Bodies of the tasks every target runs.
//!
//! `#[embassy_executor::task]` functions can't be generic, so each target
//! wraps these in a few lines of its own.

use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use edge_http::io::server::Server;
use edge_nal::{TcpBind, WithTimeout};
use embassy_time::{Duration, Timer};

use extreme_traits::RawEngine;

use crate::{
    config::{
        HTTP_KEEPALIVE_TIMEOUT_MS, MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, SOCKET_IO_TIMEOUT_MS,
    },
    fmt::Dbg,
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
                info!("http: listening on port {}", port);
                // There are only MAX_WEB_SOCKETS connection slots, and a
                // client that disappears without closing its connection
                // must not hold one forever: idle keep-alives are closed,
                // and every socket operation is bounded (which also bounds
                // the final close, otherwise stuck waiting for the client
                // to acknowledge). Websockets have their own liveness check,
                // see `HttpHandler::run_websocket`.
                let acceptor = WithTimeout::new(SOCKET_IO_TIMEOUT_MS, acceptor);
                let handler = HttpHandler::new(runtime);
                if let Err(e) = server
                    .run(Some(HTTP_KEEPALIVE_TIMEOUT_MS), acceptor, handler)
                    .await
                {
                    error!("http: server error: {:?}", Dbg(&e));
                }
            }
            Err(e) => error!("http: cannot bind port {}: {:?}", port, Dbg(&e)),
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
        debug!("gps: {:?}", Dbg(&update));
        runtime.gps_update(update).await;
    }
}
