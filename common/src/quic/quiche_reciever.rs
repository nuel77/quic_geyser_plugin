use anyhow::bail;

use crate::message::Message;

use super::configure_server::MAX_DATAGRAM_SIZE;

pub fn convert_binary_to_message(bytes: Vec<u8>) -> anyhow::Result<Message> {
    Ok(bincode::deserialize::<Message>(&bytes)?)
}

pub fn recv_message(
    connection: &mut quiche::Connection,
    stream_id: u64,
) -> anyhow::Result<Message> {
    let mut total_buf = vec![];
    loop {
        let mut buf = [0; MAX_DATAGRAM_SIZE]; // 10kk buffer size
        match connection.stream_recv(stream_id, &mut buf) {
            Ok((read, fin)) => {
                total_buf.append(&mut buf[0..read].to_vec());
                if fin {
                    return Ok(bincode::deserialize::<Message>(&total_buf)?);
                }
            }
            Err(_) => {
                bail!("Fail to read from stream {stream_id}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        str::FromStr,
        thread::sleep,
        time::Duration,
    };

    use itertools::Itertools;
    use quiche::ConnectionId;
    use ring::rand::{SecureRandom, SystemRandom};
    use std::net::UdpSocket;

    use crate::{
        message::Message,
        quic::{
            configure_client::configure_client,
            configure_server::{configure_server, MAX_DATAGRAM_SIZE},
            quiche_reciever::recv_message,
            quiche_sender::send_message,
            quiche_utils::{mint_token, validate_token},
        },
        types::account::Account,
    };

    #[test]
    fn test_send_and_recieve_of_small_account() {
        let mut config = configure_server(1, 100000, 1).unwrap();

        // Setup the event loop.
        let socket_addr = SocketAddr::from_str("0.0.0.0:0").unwrap();
        let socket = UdpSocket::bind(socket_addr).unwrap();

        let port = socket.local_addr().unwrap().port();
        let local_addr = socket.local_addr().unwrap();

        let account = Account::get_account_for_test(123456, 2);
        let message = Message::AccountMsg(account);

        let jh = {
            let message_to_send = message.clone();
            let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
            std::thread::spawn(move || {
                let mut client_config = configure_client(1, 1000, 10).unwrap();

                // Setup the event loop.
                let socket_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
                let socket = std::net::UdpSocket::bind(socket_addr).unwrap();

                let mut scid = [0; quiche::MAX_CONN_ID_LEN];
                SystemRandom::new().fill(&mut scid[..]).unwrap();

                let scid = quiche::ConnectionId::from_ref(&scid);

                // Get local address.
                let local_addr = socket.local_addr().unwrap();
                println!("connecting");
                let mut conn =
                    quiche::connect(None, &scid, local_addr, server_addr, &mut client_config)
                        .unwrap();
                let mut out = [0; MAX_DATAGRAM_SIZE];
                println!("sending message");
                let (write, send_info) = conn.send(&mut out).expect("initial send failed");

                while let Err(e) = socket.send_to(&out[..write], send_info.to) {
                    panic!("send() failed: {:?}", e);
                }

                while !conn.is_established() {
                    sleep(Duration::from_millis(100));
                }

                send_message(&mut conn, 4, &message_to_send).unwrap();

                // Generate outgoing QUIC packets and send them on the UDP socket, until
                // quiche reports that there are no more packets to be sent.
                loop {
                    let (write, send_info) = match conn.send(&mut out) {
                        Ok(v) => v,

                        Err(quiche::Error::Done) => {
                            break;
                        }

                        Err(e) => {
                            conn.close(false, 0x1, b"fail").ok();
                            break;
                        }
                    };

                    if let Err(e) = socket.send_to(&out[..write], send_info.to) {
                        if e.kind() == std::io::ErrorKind::WouldBlock {
                            break;
                        }

                        panic!("send() failed: {:?}", e);
                    }
                }
                conn.close(true, 0, b"not required").unwrap();
            })
        };

        loop {
            let mut buf = [0; 65535];
            let mut out = [0; MAX_DATAGRAM_SIZE];

            let (len, from) = match socket.recv_from(&mut buf) {
                Ok(v) => v,
                Err(e) => {
                    panic!("recv() failed: {:?}", e);
                }
            };
            println!("recieved first packet");

            log::debug!("got {} bytes", len);

            let pkt_buf = &mut buf[..len];

            // Parse the QUIC packet's header.
            let hdr = match quiche::Header::from_slice(pkt_buf, quiche::MAX_CONN_ID_LEN) {
                Ok(header) => header,

                Err(e) => {
                    panic!("Parsing packet header failed: {:?}", e);
                }
            };
            let rng = SystemRandom::new();
            let conn_id_seed = ring::hmac::Key::generate(ring::hmac::HMAC_SHA256, &rng).unwrap();
            let conn_id = ring::hmac::sign(&conn_id_seed, &hdr.dcid);
            let conn_id = &conn_id.as_ref()[..quiche::MAX_CONN_ID_LEN];
            let conn_id: ConnectionId<'static> = conn_id.to_vec().into();

            if hdr.ty != quiche::Type::Initial {
                panic!("Packet is not Initial");
            }

            if !quiche::version_is_supported(hdr.version) {
                log::warn!("Doing version negotiation");

                let len = quiche::negotiate_version(&hdr.scid, &hdr.dcid, &mut out).unwrap();

                let out = &out[..len];

                if let Err(e) = socket.send_to(out, from) {
                    panic!("send() failed: {:?}", e);
                }
            }

            let mut scid = [0; quiche::MAX_CONN_ID_LEN];
            scid.copy_from_slice(&conn_id);

            let scid = quiche::ConnectionId::from_ref(&scid);

            // Token is always present in Initial packets.
            let token = hdr.token.as_ref().unwrap();

            println!("token: {}", token.iter().map(|x| x.to_string()).join(", "));

            // Do stateless retry if the client didn't send a token.
            if token.is_empty() {
                log::warn!("Doing stateless retry");

                let new_token = mint_token(&hdr, &from);

                let len = quiche::retry(
                    &hdr.scid,
                    &hdr.dcid,
                    &scid,
                    &new_token,
                    hdr.version,
                    &mut out,
                )
                .unwrap();

                let out = &out[..len];

                if let Err(e) = socket.send_to(out, from) {
                    panic!("send() failed: {:?}", e);
                } else {
                    continue;
                }
            }
            let odcid = validate_token(&from, token);
            // The token was not valid, meaning the retry failed, so
            // drop the packet.
            if odcid.is_none() {
                panic!("Invalid address validation token");
            }

            if scid.len() != hdr.dcid.len() {
                panic!("Invalid destination connection ID");
            }

            // Reuse the source connection ID we sent in the Retry packet,
            // instead of changing it again.
            let scid = hdr.dcid.clone();

            log::debug!("New connection: dcid={:?} scid={:?}", hdr.dcid, scid);

            let mut conn =
                quiche::accept(&scid, odcid.as_ref(), local_addr, from, &mut config).unwrap();

            let stream_id = {
                let mut stream_id = 0;
                loop {
                    let readable = conn.stream_readable_next();
                    match readable {
                        Some(id) => {
                            stream_id = id;
                            break;
                        }
                        None => {
                            sleep(Duration::from_millis(100));
                        }
                    }
                }
                stream_id
            };
            let recvd_message = recv_message(&mut conn, stream_id).unwrap();
            assert_eq!(recvd_message, message);
            std::thread::sleep(Duration::from_secs(1));
            assert_eq!(conn.is_closed(), true);
            jh.join().unwrap();
            break;
        }
    }

    #[test]
    fn test_send_and_recieve_of_large_account() {
        let mut config = configure_server(1, 100000, 1).unwrap();

        // Setup the event loop.
        let socket_addr = SocketAddr::from_str("0.0.0.0:0").unwrap();
        let socket = UdpSocket::bind(socket_addr).unwrap();

        let port = socket.local_addr().unwrap().port();
        let local_addr = socket.local_addr().unwrap();

        let account = Account::get_account_for_test(123456, 10_000_000);
        let message = Message::AccountMsg(account);

        let jh = {
            let message = message.clone();
            let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
            std::thread::spawn(move || {
                let mut client_config = configure_client(1, 12_000_000, 10).unwrap();

                // Setup the event loop.
                let socket_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
                let socket = std::net::UdpSocket::bind(socket_addr).unwrap();

                let mut scid = [0; quiche::MAX_CONN_ID_LEN];
                SystemRandom::new().fill(&mut scid[..]).unwrap();

                let scid = quiche::ConnectionId::from_ref(&scid);

                // Get local address.
                let local_addr = socket.local_addr().unwrap();
                println!("connecting");
                let mut conn =
                    quiche::connect(None, &scid, local_addr, server_addr, &mut client_config)
                        .unwrap();
                let mut out = [0; MAX_DATAGRAM_SIZE];
                println!("sending message");
                let (write, send_info) = conn.send(&mut out).expect("initial send failed");

                while let Err(e) = socket.send_to(&out[..write], send_info.to) {
                    panic!("send() failed: {:?}", e);
                }

                let stream_id = conn.stream_readable_next().unwrap();
                let recvd_message = recv_message(&mut conn, stream_id).unwrap();
                assert_eq!(recvd_message, message);
                std::thread::sleep(Duration::from_secs(1));
                assert_eq!(conn.is_closed(), true);
            })
        };

        loop {
            let mut buf = [0; 65535];
            let mut out = [0; MAX_DATAGRAM_SIZE];

            let (len, from) = match socket.recv_from(&mut buf) {
                Ok(v) => v,
                Err(e) => {
                    panic!("recv() failed: {:?}", e);
                }
            };
            println!("recieved first packet");

            log::debug!("got {} bytes", len);

            let pkt_buf = &mut buf[..len];

            // Parse the QUIC packet's header.
            let hdr = match quiche::Header::from_slice(pkt_buf, quiche::MAX_CONN_ID_LEN) {
                Ok(header) => header,

                Err(e) => {
                    panic!("Parsing packet header failed: {:?}", e);
                }
            };
            let rng = SystemRandom::new();
            let conn_id_seed = ring::hmac::Key::generate(ring::hmac::HMAC_SHA256, &rng).unwrap();
            let conn_id = ring::hmac::sign(&conn_id_seed, &hdr.dcid);
            let conn_id = &conn_id.as_ref()[..quiche::MAX_CONN_ID_LEN];
            let conn_id: ConnectionId<'static> = conn_id.to_vec().into();

            if hdr.ty != quiche::Type::Initial {
                panic!("Packet is not Initial");
            }

            if !quiche::version_is_supported(hdr.version) {
                log::warn!("Doing version negotiation");

                let len = quiche::negotiate_version(&hdr.scid, &hdr.dcid, &mut out).unwrap();

                let out = &out[..len];

                if let Err(e) = socket.send_to(out, from) {
                    panic!("send() failed: {:?}", e);
                }
            }

            let mut scid = [0; quiche::MAX_CONN_ID_LEN];
            scid.copy_from_slice(&conn_id);

            let scid = quiche::ConnectionId::from_ref(&scid);

            // Token is always present in Initial packets.
            let token = hdr.token.as_ref().unwrap();

            println!("token: {}", token.iter().map(|x| x.to_string()).join(", "));

            // Do stateless retry if the client didn't send a token.
            if token.is_empty() {
                log::warn!("Doing stateless retry");

                let new_token = mint_token(&hdr, &from);

                let len = quiche::retry(
                    &hdr.scid,
                    &hdr.dcid,
                    &scid,
                    &new_token,
                    hdr.version,
                    &mut out,
                )
                .unwrap();

                let out = &out[..len];

                if let Err(e) = socket.send_to(out, from) {
                    panic!("send() failed: {:?}", e);
                } else {
                    continue;
                }
            }
            let odcid = validate_token(&from, token);
            // The token was not valid, meaning the retry failed, so
            // drop the packet.
            if odcid.is_none() {
                panic!("Invalid address validation token");
            }

            if scid.len() != hdr.dcid.len() {
                panic!("Invalid destination connection ID");
            }

            // Reuse the source connection ID we sent in the Retry packet,
            // instead of changing it again.
            let scid = hdr.dcid.clone();

            log::debug!("New connection: dcid={:?} scid={:?}", hdr.dcid, scid);

            let mut conn =
                quiche::accept(&scid, odcid.as_ref(), local_addr, from, &mut config).unwrap();

            let stream_id = conn.stream_writable_next().unwrap();
            send_message(&mut conn, stream_id, &message).unwrap();
            conn.close(true, 0, b"not required").unwrap();

            jh.join().unwrap();
            break;
        }
    }
}
