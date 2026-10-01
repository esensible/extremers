use embassy_net::{Runner, Stack};
use embassy_time::{Duration, Timer};

use cyw43_pio::PioSpi;
use embassy_rp::gpio::Output;
use embassy_rp::peripherals::PIO0;

use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use edge_dhcp::io::DEFAULT_SERVER_PORT;
use edge_dhcp::server::ServerOptions;
use edge_nal::UdpBind;
use edge_nal_embassy::{Udp, UdpBuffers};

#[embassy_executor::task]
pub async fn dhcp_server_task(stack: Stack<'static>, ip: Ipv4Addr) -> ! {
    let buffers = UdpBuffers::<1, 1500, 1500, 2>::new();
    let udp = Udp::new(stack, &buffers);

    let mut socket = match udp
        .bind(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            DEFAULT_SERVER_PORT,
        ))
        .await
    {
        Ok(socket) => socket,
        Err(e) => {
            log::error!("Failed to bind DHCP server socket: {:?}", e);
            loop {
                Timer::after(Duration::from_secs(1)).await;
                log::error!("DHCP server loop");
            }
        }
    };

    // Will give IP addresses in the range x.x.x.50 - x.x.x.200, subnet 255.255.255.0
    let mut server = edge_dhcp::server::Server::<_, 64>::new_with_et(ip);
    let mut gw_buf = [ip];
    let mut server_options = ServerOptions::new(ip, Some(&mut gw_buf));
    let dns_servers = [ip];
    server_options.dns = &dns_servers;

    let mut buf = [0u8; 1500];

    loop {
        if let Err(e) =
            edge_dhcp::io::server::run(&mut server, &server_options, &mut socket, &mut buf).await
        {
            log::warn!("DHCP server error: {:?}", e);
            Timer::after(Duration::from_secs(1)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn wifi_task(
    runner: cyw43::Runner<'static, cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}
