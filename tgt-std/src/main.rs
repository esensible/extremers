// Standard library imports
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

// Embassy framework imports
use embassy_executor::Executor;
// use embassy_net::Stack;
use embassy_time::{Duration, Timer};

// Networking imports
use edge_net::{
    // embassy::{Tcp, TcpBuffers},
    http::io::server::Server,
    nal::TcpBind,
    std::Stack,
};

// Other external crates
use static_cell::StaticCell;

// Local modules
mod http;

use crate::http::HttpHandler;

use extreme_traits::define_engines;

// type EngineType = extreme_race::Race;

define_engines! {
    EngineType {
        Race(extreme_race::Race),
        TuneSpeed(extreme_tune::TuneSpeed<32>),
        Map(extreme_map::RaceMap),
    }
}

// Constants
const MAX_WEB_SOCKETS: usize = 4;
const MAX_MESSAGE_SIZE: usize = 512;
const SOCKET_BUFFER_SIZE: usize = MAX_MESSAGE_SIZE * 4;

// env_logger::builder()
//     .filter_level(log::LevelFilter::Debug)
//     .filter_module("async_io", log::LevelFilter::Info)
//     .format_timestamp_nanos()
//     .init();

fn main() {
    static HTTPD_HANDLER: StaticCell<HttpHandler<EngineType>> = StaticCell::new();
    let httpd_handler = HTTPD_HANDLER.init(HttpHandler::new(EngineType::default()));

    // Init network stack
    static STACK: StaticCell<Stack> = StaticCell::new();
    let stack = Stack::new();
    let stack = STACK.init(stack);

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        let result = spawner.spawn(httpd_task(stack, httpd_handler));
        if result.is_err() {
            log::warn!("failed to spawn httpd task");
        }

        let result = spawner.spawn(sleeper_task(httpd_handler));
        if result.is_err() {
            log::warn!("failed to spawn sleeper task");
        }
    });
}

#[embassy_executor::task]
pub async fn sleeper_task(handler: &'static HttpHandler<EngineType>) {
    handler.run_sleeper().await
}

#[embassy_executor::task]
pub async fn httpd_task(stack: &'static Stack, handler: &'static HttpHandler<EngineType>) -> ! {
    // let buffers = TcpBuffers::<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, SOCKET_BUFFER_SIZE>::new();
    // let tcp = Tcp::new(stack, &buffers);

    loop {
        let acceptor = match stack
            .bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8080))
            .await
        {
            Ok(socket) => socket,
            Err(e) => {
                log::error!("Failed to bind httpd socket: {:?}", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        };

        let mut server: Server<MAX_WEB_SOCKETS, SOCKET_BUFFER_SIZE, 64> = Server::new();
        match server.run(None, acceptor, handler).await {
            Ok(_) => (),
            Err(e) => {
                log::error!("HTTPd server error: {:?}", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        }
    }
}
