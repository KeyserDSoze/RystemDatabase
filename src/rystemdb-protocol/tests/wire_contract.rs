use rystemdb_protocol::{
    decode, encode, read_frame, read_frame_with_limit, write_frame, Frame, Kind, Operation, ProtocolError,
    HEADER_LEN, MAX_PAYLOAD_BYTES,
};
use std::io::Cursor;

#[test]
fn tdd_ping_golden_big_endian_octets() {
    let frame = Frame::request(Operation::Ping, 7, Vec::new());
    let bytes = encode(&frame).unwrap();
    assert_eq!(bytes, [
        0x52,0x53,0x57,0x50, // RSWP
        1,0,               // major/minor
        1,2,               // request/ping
        0,0,               // status/reserved
        0,24,              // header size (big-endian u16)
        0,0,0,0,0,0,0,7,   // request id (big-endian u64)
        0,0,0,0,           // body len (big-endian u32)
    ]);
    assert_eq!(bytes.len(), HEADER_LEN);
    assert_eq!(decode(&bytes), Ok(frame));
}

#[test]
fn tdd_binary_payload_roundtrip_is_not_json() {
    let body = b"RDB\0\x01\x08\x00\x00\x00\x00".to_vec();
    let frame = Frame::request(Operation::Put, u64::MAX, body.clone());
    let bytes = encode(&frame).unwrap();
    assert!(bytes.starts_with(b"RSWP"));
    assert!(!bytes.starts_with(b"{"));
    assert_eq!(&bytes[HEADER_LEN..], body.as_slice());
    assert_eq!(decode(&bytes), Ok(frame));
}

#[test]
fn tdd_request_and_response_status_are_distinct() {
    let mut req = Frame::request(Operation::Get, 42, Vec::new());
    req.status = 1;
    assert_eq!(encode(&req), Err(ProtocolError::InvalidStatus));
    let response = Frame { kind: Kind::Response, operation: Operation::Get,
        request_id: 42, status: 1, payload: vec![4,5,6] };
    assert_eq!(decode(&encode(&response).unwrap()), Ok(response));
}

#[test]
fn tdd_rejects_truncation_at_every_byte() {
    let input = encode(&Frame::request(Operation::Put, 12, vec![0,1,2,3,4,5])).unwrap();
    for i in 0..input.len() {
        assert_eq!(decode(&input[..i]), Err(ProtocolError::Incomplete),
            "frame truncated at offset {i}");
    }
}

#[test]
fn tdd_rejects_malformed_version_kind_opcode_and_header() {
    let valid = encode(&Frame::request(Operation::Ping, 7, Vec::new())).unwrap();
    let mut bad = valid.clone();
    bad[0] = b'X';
    assert_eq!(decode(&bad), Err(ProtocolError::InvalidMagic));
    let mut bad = valid.clone();
    bad[4] = 2;
    assert_eq!(decode(&bad), Err(ProtocolError::UnsupportedVersion));
    let mut bad = valid.clone();
    bad[6] = 9;
    assert_eq!(decode(&bad), Err(ProtocolError::InvalidKind));
    let mut bad = valid.clone();
    bad[7] = 99;
    assert_eq!(decode(&bad), Err(ProtocolError::InvalidOperation));
    let mut bad = valid.clone();
    bad[10] = 0;
    bad[11] = 25;
    assert_eq!(decode(&bad), Err(ProtocolError::InvalidHeaderLength));
}

#[test]
fn tdd_validates_length_before_allocation_and_rejects_extra_data() {
    let mut huge = vec![0; HEADER_LEN];
    huge[..4].copy_from_slice(b"RSWP");
    huge[4] = 1;
    huge[6] = 1;
    huge[7] = 2;
    huge[11] = HEADER_LEN as u8;
    huge[20..24].copy_from_slice(&((MAX_PAYLOAD_BYTES as u32) + 1).to_be_bytes());
    assert_eq!(decode(&huge), Err(ProtocolError::TooLarge));
    let mut valid = encode(&Frame::request(Operation::Ping, 9, Vec::new())).unwrap();
    valid.push(1);
    assert_eq!(decode(&valid), Err(ProtocolError::TrailingBytes));
}

#[test]
fn tdd_streaming_frame_reader_handles_chunked_reads() {
    let first = Frame::request(Operation::Hello, 1, vec![1, 0]);
    let second = Frame::request(Operation::Ping, 2, Vec::new());
    let mut stream = Vec::new();
    write_frame(&mut stream, &first).unwrap();
    write_frame(&mut stream, &second).unwrap();
    let mut input = Cursor::new(stream);
    assert_eq!(read_frame(&mut input), Ok(first));
    assert_eq!(read_frame(&mut input), Ok(second));
    assert_eq!(read_frame(&mut input), Err(ProtocolError::Incomplete));
}

#[test]
fn tdd_empty_body_and_large_id_roundtrip() {
    for op in [Operation::Hello, Operation::Ping, Operation::Get, Operation::Delete] {
        let frame = Frame::request(op, u64::MAX, Vec::new());
        assert_eq!(decode(&encode(&frame).unwrap()), Ok(frame));
    }
}


// #72: a pre-authentication protocol consumer can cap frame bodies at the
// authenticated HELLO length, without changing normal post-HELLO CRUD.
#[test]
fn tdd_bounded_stream_reader_checks_header_before_touching_payload() {
    let authenticated_hello = Frame::request(Operation::Hello, 1, {
        let mut body = vec![1, 0, 0, 32];
        body.extend_from_slice(&[0x11; 32]);
        body
    });
    let wire = encode(&authenticated_hello).unwrap();
    assert_eq!(wire.len(), HEADER_LEN + 36);
    assert_eq!(
        read_frame_with_limit(&mut Cursor::new(wire.clone()), 36),
        Ok(authenticated_hello),
        "exact 36-byte bearer HELLO must remain readable"
    );
    let mut restricted = Cursor::new(wire);
    assert_eq!(read_frame_with_limit(&mut restricted, 35), Err(ProtocolError::TooLarge));
    assert_eq!(restricted.position(), HEADER_LEN as u64,
        "overbudget HELLO must be rejected after header, without a body read");

    // The header claims 16 MiB, but no body was sent at all. Must return
    // TooLarge rather than Incomplete or waiting to consume the body.
    let mut header = encode(&Frame::request(Operation::Hello, 2, vec![])).unwrap();
    header[20..24].copy_from_slice(&(MAX_PAYLOAD_BYTES as u32).to_be_bytes());
    let mut malicious = Cursor::new(header);
    assert_eq!(
        read_frame_with_limit(&mut malicious, 36),
        Err(ProtocolError::TooLarge),
        "pre-auth length cap must take precedence over a missing body"
    );
    assert_eq!(malicious.position(), HEADER_LEN as u64);

    // A complete 37-byte body must also be rejected *without consuming it*.
    let mut too_many = Cursor::new(encode(
        &Frame::request(Operation::Hello, 3, vec![0; 37])
    ).unwrap());
    assert_eq!(read_frame_with_limit(&mut too_many, 36), Err(ProtocolError::TooLarge));
    assert_eq!(too_many.position(), HEADER_LEN as u64);

    // The original authenticated/post-HELLO reader still accepts frames
    // above 36 bytes and preserves their complete opaque payload.
    let normal = Frame::request(Operation::Put, 4, vec![0x80; 4096]);
    assert_eq!(read_frame(&mut Cursor::new(encode(&normal).unwrap())), Ok(normal));
}

#[test]
fn tdd_bounded_stream_reader_preserves_header_errors_and_multi_frame_streams() {
    let mut payload = Vec::new();
    write_frame(&mut payload, &Frame::request(Operation::Hello, 1, vec![1,0])).unwrap();
    write_frame(&mut payload, &Frame::request(Operation::Ping, 2, vec![])).unwrap();
    let mut input = Cursor::new(payload);
    assert_eq!(read_frame_with_limit(&mut input, 36),
        Ok(Frame::request(Operation::Hello, 1, vec![1,0])));
    assert_eq!(read_frame_with_limit(&mut input, 0),
        Ok(Frame::request(Operation::Ping, 2, vec![])));
    assert_eq!(read_frame_with_limit(&mut input, 36), Err(ProtocolError::Incomplete));

    let mut invalid = encode(&Frame::request(Operation::Hello, 1, vec![1,0])).unwrap();
    invalid[4] = 2;
    assert_eq!(read_frame_with_limit(&mut Cursor::new(invalid), 36),
        Err(ProtocolError::UnsupportedVersion));
}
