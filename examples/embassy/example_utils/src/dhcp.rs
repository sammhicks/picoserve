use embassy_net::{wire::Ipv4Address, Stack, udp::UdpSocket};

pub const SOCKET_COUNT: usize = 1;

#[embassy_executor::task]
pub async fn dhcp_task(address: Ipv4Address, stack: Stack<'static>) {
    let mut socket = UdpSocket::new(stack).unwrap();

    socket.bind(67).expect("Failed to bind DHCP Socket");

    let server_options = edge_dhcp::server::ServerOptions::new(address, None);

    let mut dhcp_server = edge_dhcp::server::Server::<_, 16>::new_with_et(address);

    let mut udp_buffer = [0; 1024];

    loop {
        let Ok((read_size, remote_endpoint)) = socket.recv_from(&mut udp_buffer).await.map_err(
            |error: embassy_net::udp::RecvError| {
                log_warn!("Failed to recv UDP packet: {:?}", error);
            },
        ) else {
            continue;
        };

        let Ok(request) = edge_dhcp::Packet::decode(&udp_buffer[..read_size]).map_err(|error| {
            log_warn!(
                "Failed to decode dhcp packet from {:?}: {:?}",
                remote_endpoint,
                error
            );
        }) else {
            continue;
        };

        let mut opt_buf = edge_dhcp::Options::buf();

        if let Some(reply) = dhcp_server.handle_request(&mut opt_buf, &server_options, &request) {
            let Ok(response) = reply.encode(&mut udp_buffer).map_err(|error| {
                log_warn!("Failed to encode dhcp packet: {:?}", error);
            }) else {
                continue;
            };

            let Ok(()) = socket
                .send_to(
                    response,
                    embassy_net::wire::IpEndpoint {
                        addr: Ipv4Address::BROADCAST.into(),
                        port: remote_endpoint.endpoint.port,
                    },
                )
                .await
                .map_err(|error| {
                    log_warn!("Failed to send dhcp packet: {:?}", error);
                })
            else {
                continue;
            };
        }
    }
}
