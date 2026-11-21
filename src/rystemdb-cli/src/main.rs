//! Native Rystem Database CLI: HTTP health, local document CRUD and loopback RSWP RPC.
mod auth;
mod json_codec;
mod rpc;

use rystemdb_storage::DocumentStore;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

const MAX_JSON_INPUT: u64 = 64 * 1024 * 1024;

fn usage() -> &'static str {
    "Rystem Database (experimental)\n\
Usage:\n\
  rystemdb --version\n\
  rystemdb document put --data PATH --collection NAME --id ID --json - [--typed-json]\n\
  rystemdb document get --data PATH --collection NAME --id ID\n\
  rystemdb document delete --data PATH --collection NAME --id ID\n\
  rystemdb serve [--listen 127.0.0.1:8787]\n\
  rystemdb rpc-serve --data PATH [--listen 127.0.0.1:8790] [--auth-file PRIVATE_PATH]\n\
  rystemdb --help\n\
PUT accepts JSON from stdin (--json -) or a local file (--json FILE).\n\
--typed-json explicitly interprets $rystem tagged references/bytes.\n\
The CLI persists binary documents; HTTP serve exposes health only; rpc-serve has local binary CRUD.\n\
RPC is experimental localhost-only; --auth-file enables private-token collection ACLs. No TLS.\n\
No SQL, TLS 1.3, secure remote authentication, distributed RPC, or multi-document transactions yet.\n"
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

struct DocumentArgs<'a> {
    data: &'a str,
    collection: &'a str,
    id: &'a str,
    json: Option<&'a str>,
    typed: bool,
}

fn parse_document_args<'a>(operation: &str, args: &'a [String]) -> Result<DocumentArgs<'a>, String> {
    if !matches!(operation, "put" | "get" | "delete") {
        return Err("document operation must be put, get or delete".into());
    }
    let (mut data, mut collection, mut id, mut json) = (None, None, None, None);
    let mut typed = false;
    let mut pos = 0;
    while pos < args.len() {
        let flag = args[pos].as_str();
        if flag == "--typed-json" {
            if typed { return Err("duplicate --typed-json".into()); }
            typed = true;
            pos += 1;
            continue;
        }
        let Some(value) = args.get(pos + 1).map(String::as_str) else {
            return Err(format!("missing value for {flag}"));
        };
        let destination = match flag {
            "--data" => &mut data,
            "--collection" => &mut collection,
            "--id" => &mut id,
            "--json" => &mut json,
            _ => return Err(format!("unknown document argument: {flag}")),
        };
        if destination.replace(value).is_some() {
            return Err(format!("duplicate argument: {flag}"));
        }
        pos += 2;
    }
    let data = data.ok_or("--data is required")?;
    let collection = collection.ok_or("--collection is required")?;
    let id = id.ok_or("--id is required")?;
    if data.is_empty() { return Err("--data cannot be empty".into()); }
    if collection.is_empty() || collection.len() > 128
        || !collection.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("collection must be 1..=128 ASCII letters, digits, hyphens or underscores".into());
    }
    if id.is_empty() || id.bytes().any(|b| b.is_ascii_control())
        || collection.len() + 1 + id.len() > 1024
    {
        return Err("document ID is empty, too long, or contains control characters".into());
    }
    if operation == "put" && json.is_none() { return Err("put requires --json - or --json FILE".into()); }
    if operation != "put" && (json.is_some() || typed) {
        return Err("--json and --typed-json are only available for put".into());
    }
    Ok(DocumentArgs { data, collection, id, json, typed })
}

fn read_json(source: &str) -> Result<String, String> {
    let mut data = Vec::new();
    if source == "-" {
        std::io::stdin().take(MAX_JSON_INPUT + 1)
            .read_to_end(&mut data).map_err(|e| format!("cannot read stdin: {e}"))?;
    } else {
        if fs::metadata(source).map_err(|e| format!("cannot stat input: {e}"))?.len() > MAX_JSON_INPUT {
            return Err("JSON input is too large".into());
        }
        data = fs::read(source).map_err(|e| format!("cannot read JSON file: {e}"))?;
    }
    if data.len() as u64 > MAX_JSON_INPUT { return Err("JSON input is too large".into()); }
    String::from_utf8(data).map_err(|_| "input JSON must be UTF-8".into())
}

fn document_command(args: &[String]) -> Result<(), String> {
    let Some((operation, rest)) = args.split_first() else {
        return Err("document requires put, get or delete".into());
    };
    let options = parse_document_args(operation, rest)?;
    let key = format!("{}/{}", options.collection, options.id);
    match operation.as_str() {
        "put" => {
            // Parse and validate before touching any database file.
            let raw = read_json(options.json.expect("put requires --json"))?;
            let value = json_codec::parse_document(&raw, options.typed)?;
            let mut store = DocumentStore::open(options.data)
                .map_err(|e| format!("cannot open database: {e}"))?;
            store.put(&key, &value).map_err(|e| format!("cannot write document: {e}"))?;
            println!("{{\"ok\":true}}");
            Ok(())
        }
        "get" => {
            let store = DocumentStore::open(options.data)
                .map_err(|e| format!("cannot open database: {e}"))?;
            let value = store.get(&key).map_err(|e| format!("cannot read document: {e}"))?
                .ok_or_else(|| format!("document not found: {key}"))?;
            println!("{}", json_codec::render_document(&value)?);
            Ok(())
        }
        "delete" => {
            let mut store = DocumentStore::open(options.data)
                .map_err(|e| format!("cannot open database: {e}"))?;
            if !store.delete(&key).map_err(|e| format!("cannot delete document: {e}"))? {
                return Err(format!("document not found: {key}"));
            }
            println!("{{\"deleted\":true}}");
            Ok(())
        }
        _ => unreachable!("operation already validated"),
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args {
        [] => { print!("{}", usage()); Ok(()) }
        [command] if command == "--help" || command == "-h" => {
            print!("{}", usage()); Ok(())
        }
        [command] if command == "--version" || command == "version" => {
            println!("rystemdb {}", env!("CARGO_PKG_VERSION")); Ok(())
        }
        [command, rest @ ..] if command == "document" => document_command(rest),
        [command, rest @ ..] if command == "rpc-serve" => rpc::run(rest),
        [command, rest @ ..] if command == "serve" => {
            let addr = match rest {
                [] => "127.0.0.1:8787",
                [flag, addr] if flag == "--listen" => addr.as_str(),
                _ => return Err("invalid serve arguments; see --help".into()),
            };
            let listener = TcpListener::bind(addr).map_err(|err| err.to_string())?;
            println!("Rystem Database experimental health server listening on {}",
                listener.local_addr().map_err(|err| err.to_string())?);
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
