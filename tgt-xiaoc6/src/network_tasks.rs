use embassy_time::{Duration, Timer};

use embassy_net::{Runner, Stack};

use esp_radio::wifi::{Interface, WifiController, ap::EventInfo};

use core::str::FromStr;

#[embassy_executor::task]
pub async fn dhcp_task(stack: Stack<'static>, gw_ip_addr: &'static str) {
    use core::net::{Ipv4Addr, SocketAddrV4};

    use edge_dhcp::{
        io::{self, DEFAULT_SERVER_PORT},
        server::{Server, ServerOptions},
    };
    use edge_nal::UdpBind;
    use edge_nal_embassy::{Udp, UdpBuffers};

    let ip = Ipv4Addr::from_str(gw_ip_addr).expect("dhcp task failed to parse gw ip");

    let mut buf = [0u8; 1500];

    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];
    let mut server_options = ServerOptions::new(ip, Some(&mut gw_buf));
    let dns_servers = [ip];
    server_options.dns = &dns_servers;

    let buffers = UdpBuffers::<2, 1024, 1024, 10>::new();
    // let buffers = UdpBuffers::<1, 1024, 1024, 2>::new();

    let unbound_socket = Udp::new(stack, &buffers);
    let mut bound_socket = unbound_socket
        .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
        .unwrap();

    loop {
        _ = io::server::run(
            &mut Server::<_, 64>::new_with_et(ip),
            &server_options,
            &mut bound_socket,
            &mut buf,
        )
        .await
        .inspect_err(|e| log::warn!("DHCP server error: {e:?}"));
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
pub async fn wifi_task(controller: WifiController<'static>) {
    // The access point is configured and started by `WifiController::new`,
    // so all that is left to do here is report on stations coming and going.
    log::debug!("start connection task");
    loop {
        match controller
            .wait_for_access_point_connected_event_async()
            .await
        {
            Ok(EventInfo::Connected(info)) => log::info!("Station connected: {:?}", info),
            Ok(EventInfo::Disconnected(info)) => log::info!("Station disconnected: {:?}", info),
            Err(e) => {
                log::warn!("Wifi event error: {:?}", e);
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
