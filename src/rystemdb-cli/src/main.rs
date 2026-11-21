//! First foundation CLI. No storage, query, or external-network authentication yet.
use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

fn usage() -> &'static str {
    "Rystem Database (foundation preview)\n\
Usage:\n\
  rystemdb --version\n\
  rystemdb serve [--listen 127.0.0.1:8787]\n\
  rystemdb --help\n\
The foundation preview exposes only /healthz and /readyz. No data is stored.\n"
}

fn response_for(request: &[u8]) -> &'static [u8] {
    if request.starts_with(b"GET /healthz HTTP/") || request.starts_with(b"GET /readyz HTTP/") {
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"status\":\"ok\"}"
    } else {
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    }
}

fn handle_connection(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut input = [0u8; 4096];
    let count = stream.read(&mut input)?;
    if count != 0 {
        stream.write_all(response_for(&input[..count]))?;
    }
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    match args {
        [] | [_,] if args.is_empty() => { print!("{}", usage()); Ok(()) },
        [command] if command == "--help" || command == "-h" => {
            print!("{}", usage()); Ok(())
        }
        [command] if command == "--version" || command == "version" => {
            println!("rystemdb {}", env!("CARGO_PKG_VERSION")); Ok(())
        }
        [command, rest @ ..] if command == "serve" => {
            let addr = match rest {
                [] => "127.0.0.1:8787",
                [flag, addr] if flag == "--listen" => addr.as_str(),
                _ => return Err("invalid serve arguments; see --help".into()),
            };
            let listener = TcpListener::bind(addr).map_err(|err| err.to_string())?;
            println!("Rystem Database foundation listening on {}", listener.local_addr().map_err(|err| err.to_string())?);
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        if let Err(err) = handle_connection(stream) {
                            eprintln!("connection error: {err}");
                        }
                    }
                    Err(err) => eprintln!("accept error: {err}"),
                }
            }
            Ok(())
        }
        _ => Err("unsupported command; see rystemdb --help".into()),
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(message) = run(&args) {
        eprintln!("{message}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_route_returns_expected_body_and_length() {
        let response = response_for(b"GET /healthz HTTP/1.1\r\n\r\n");
        assert!(response.starts_with(b"HTTP/1.1 200 OK"));
        assert!(response.ends_with(b"{\"status\":\"ok\"}"));
    }

    #[test]
    fn unknown_endpoint_is_not_successful() {
        assert!(response_for(b"GET /items HTTP/1.1\r\n\r\n").starts_with(b"HTTP/1.1 404"));
    }

    #[test]
    fn unsupported_command_fails_closed() {
        assert!(run(&["query".into()]).is_err());
    }
}
