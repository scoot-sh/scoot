#![cfg(unix)]

use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::thread;

use scoot_ipc::{Client, PROTOCOL_VERSION, Request, Response, read_message, write_message};

#[test]
fn client_sends_a_request_and_reads_the_reply() {
    let (client_end, server_end) = UnixStream::pair().unwrap();
    let server = thread::spawn(move || {
        let mut reader = BufReader::new(server_end.try_clone().unwrap());
        let mut writer = server_end;
        let request: Request = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(request, Request::Version);
        let reply = Response::Version {
            version: "test".into(),
            protocol: PROTOCOL_VERSION,
        };
        write_message(&mut writer, &reply).unwrap();
    });

    let mut client = Client::from_stream(client_end).unwrap();
    let response = client.request(&Request::Version).unwrap();
    assert_eq!(
        response,
        Response::Version {
            version: "test".into(),
            protocol: PROTOCOL_VERSION
        }
    );
    server.join().unwrap();
}

#[test]
fn a_client_can_send_many_requests_reusing_its_read_buffer() {
    // Client reuses one String across calls instead of allocating fresh per
    // request; this is the regression test for that -- if clearing it were
    // ever dropped, a shorter reply following a longer one would leave
    // trailing garbage from the previous line and fail to decode.
    let (client_end, server_end) = UnixStream::pair().unwrap();
    let server = thread::spawn(move || {
        let mut reader = BufReader::new(server_end.try_clone().unwrap());
        let mut writer = server_end;
        for _ in 0..3 {
            let request: Request = read_message(&mut reader).unwrap().unwrap();
            let reply = match request {
                Request::Version => Response::Version {
                    version: "test".into(),
                    protocol: PROTOCOL_VERSION,
                },
                Request::Outputs => Response::Outputs { outputs: vec![] },
                Request::Windows => Response::Windows { windows: vec![] },
                other => panic!("unexpected request {other:?}"),
            };
            write_message(&mut writer, &reply).unwrap();
        }
    });

    let mut client = Client::from_stream(client_end).unwrap();
    assert_eq!(
        client.request(&Request::Version).unwrap(),
        Response::Version {
            version: "test".into(),
            protocol: PROTOCOL_VERSION
        }
    );
    assert_eq!(
        client.request(&Request::Outputs).unwrap(),
        Response::Outputs { outputs: vec![] }
    );
    assert_eq!(
        client.request(&Request::Windows).unwrap(),
        Response::Windows { windows: vec![] }
    );
    server.join().unwrap();
}

#[test]
fn a_closed_connection_is_an_error_not_a_hang() {
    let (client_end, server_end) = UnixStream::pair().unwrap();
    drop(server_end);
    let mut client = Client::from_stream(client_end).unwrap();
    assert!(client.request(&Request::Version).is_err());
}
