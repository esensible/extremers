use defmt::{Debug2Format, debug, error, info, warn};
use embassy_time::{Duration, Timer};

use embassy_net::{Runner, Stack};

use esp_radio::wifi::{
    AuthenticationMethodConfig, Config as WifiConfig, Interface, WifiController,
    ap::AccessPointConfig, event::EventInfo,
};

use common::config::{AP_IP, CAPTIVE_URL, WIFI_CHANNEL, WIFI_PASSWORD, WIFI_SSID};

/// The access point's configuration: applied by `WifiController::new`, and
/// again by [`wifi_task`] if the access point stops.
pub fn access_point_config() -> WifiConfig {
    WifiConfig::AccessPoint(
        AccessPointConfig::default()
            .with_ssid(WIFI_SSID.try_into().unwrap())
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                WIFI_PASSWORD.try_into().unwrap(),
            ))
            .with_channel(WIFI_CHANNEL),
    )
}

#[embassy_executor::task]
pub async fn dhcp_task(stack: Stack<'static>) {
    use core::net::{Ipv4Addr, SocketAddrV4};

    use edge_dhcp::{
        io::{self, DEFAULT_SERVER_PORT},
        server::{Server, ServerOptions},
    };
    use edge_nal::UdpBind;
    use edge_nal_embassy::{Udp, UdpBuffers};

    let ip = AP_IP;

    let mut buf = [0u8; 1500];

    // overwritten with `ip` by ServerOptions::new
    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];
    let mut server_options = ServerOptions::new(ip, Some(&mut gw_buf));
    // ourselves as DNS (dns_task answers every name with `ip`), and the
    // race page as the captive portal (option 114)
    let dns_servers = [ip];
    server_options.dns = &dns_servers;
    server_options.captive_url = Some(CAPTIVE_URL);

    let buffers = UdpBuffers::<2, 1024, 1024, 10>::new();

    let unbound_socket = Udp::new(stack, &buffers);
    let mut bound_socket = loop {
        match unbound_socket
            .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::UNSPECIFIED,
                DEFAULT_SERVER_PORT,
            )))
            .await
        {
            Ok(socket) => break socket,
            Err(e) => {
                error!("DHCP server cannot bind, retrying: {:?}", Debug2Format(&e));
                Timer::after(Duration::from_millis(1000)).await;
            }
        }
    };

    // The server holds the lease table, so it outlives errors: a new one
    // would hand out addresses that clients still hold.
    let mut server = Server::<_, 64>::new_with_et(ip);
    loop {
        _ = io::server::run(&mut server, &server_options, &mut bound_socket, &mut buf)
            .await
            .inspect_err(|e| warn!("DHCP server error: {:?}", Debug2Format(e)));
        Timer::after(Duration::from_millis(500)).await;
    }
}

/// Captive DNS: every query, whatever the name, is answered with our own
/// address, so any URL typed on the Kindle lands on the race page.
/// edge-captive does the protocol; this binds UDP 53 and restarts it on
/// error.
#[embassy_executor::task]
pub async fn dns_task(stack: Stack<'static>) {
    use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

    use edge_nal_embassy::{Udp, UdpBuffers};

    let buffers = UdpBuffers::<2, 512, 512, 4>::new();
    let udp = Udp::new(stack, &buffers);
    let mut tx_buf = [0u8; 512];
    let mut rx_buf = [0u8; 512];
    info!("dns: captive resolver on :53, every name -> {}", Debug2Format(&AP_IP));
    loop {
        if let Err(e) = edge_captive::io::run(
            &udp,
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 53)),
            &mut tx_buf,
            &mut rx_buf,
            AP_IP,
            core::time::Duration::from_secs(60),
        )
        .await
        {
            warn!("dns: {:?}", Debug2Format(&e));
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
pub async fn wifi_task(mut controller: WifiController<'static>) {
    // The access point is configured and started by `WifiController::new`,
    // so this reports on stations coming and going, and restarts the access
    // point should it ever stop.
    debug!("start connection task");
    loop {
        // the subscriber borrows the controller, so it is dropped before
        // the restart below
        match controller.subscribe() {
            Ok(mut events) => loop {
                match events.next_event_pure().await {
                    EventInfo::AccessPointStop => break,
                    event @ (EventInfo::AccessPointStationConnected { .. }
                    | EventInfo::AccessPointStationDisconnected { .. }) => {
                        info!("Wifi: {:?}", Debug2Format(&event))
                    }
                    _ => {}
                }
            },
            Err(e) => {
                warn!("Wifi: cannot subscribe to events: {:?}", Debug2Format(&e));
                Timer::after(Duration::from_millis(5000)).await;
                continue;
            }
        }

        // esp-radio (1.0.0-beta.1) has no start(): it starts Wi-Fi when
        // set_config changes the mode. Each of its paths to esp_wifi_stop
        // also leaves the mode other than access point (another mode, or
        // none after a failed set_config), so setting the access point
        // configuration again starts it.
        warn!("Wifi: access point stopped, restarting");
        while let Err(e) = controller.set_config(&access_point_config()) {
            error!("Wifi: cannot restart access point: {:?}", Debug2Format(&e));
            Timer::after(Duration::from_millis(5000)).await;
        }
        info!("Wifi: access point restarted");
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
