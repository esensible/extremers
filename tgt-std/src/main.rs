//! Host target: the package host, with the same packages as the firmware
//! run under wasm3 built for this machine, serving the UIs on port 8080,
//! with no GPS.

use edge_nal_std::Stack;
use embassy_executor::Executor;
use static_cell::StaticCell;

use common::{
    host::{Host, Registry},
    tasks::serve_http,
};
use extreme_packages::PACKAGES;

/// Port 80 needs privileges on a host, so use the conventional alternative.
const HTTP_PORT: u16 = 8080;

fn main() {
    // RUST_LOG=debug for the host's timer and GPS tracing
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    static HOST: StaticCell<Host> = StaticCell::new();
    let host: &'static Host = HOST.init(Host::new(Registry::new(
        Some(&PACKAGES.choose),
        PACKAGES.preinstalled,
    )));

    static STACK: StaticCell<Stack> = StaticCell::new();
    let stack: &'static Stack = STACK.init(Stack::new());

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        match httpd_task(stack, host) {
            Ok(token) => spawner.spawn(token),
            Err(_) => log::warn!("failed to spawn httpd task"),
        }

        match timer_task(host) {
            Ok(token) => spawner.spawn(token),
            Err(_) => log::warn!("failed to spawn timer task"),
        }
    });
}

#[embassy_executor::task]
async fn httpd_task(stack: &'static Stack, host: &'static Host) -> ! {
    serve_http(stack, HTTP_PORT, host).await
}

#[embassy_executor::task]
async fn timer_task(host: &'static Host) -> ! {
    host.run_timer().await
}
