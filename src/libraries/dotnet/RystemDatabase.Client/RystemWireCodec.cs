using System.Buffers.Binary;

namespace RystemDatabase.Client;

/// <summary>
/// RSWP v1 framing contract; no network server, TLS or authentication is
/// provided by this codec alone. All fields are big-endian on the wire.
/// </summary>
public enum WireKind : byte { Request = 1, Response = 2 }
public enum WireOperation : byte { Hello = 1, Ping = 2, Put = 16, Get = 17, Delete = 18 }

public readonly record struct WireFrame(
    WireKind Kind, WireOperation Operation, ulong RequestId, ushort Status, byte[] Payload);

public static class RystemWireCodec
{
    public const int HeaderLength = 24;
    public const int MaxPayloadBytes = 16 * 1024 * 1024;

    public static byte[] Encode(WireFrame frame)
    {
        Validate(frame.Kind, frame.Operation, frame.Status);
        ArgumentNullException.ThrowIfNull(frame.Payload);
        if (frame.Payload.Length > MaxPayloadBytes)
            throw new ArgumentException("Frame payload too large", nameof(frame));

        byte[] bytes = new byte[HeaderLength + frame.Payload.Length];
        "RSWP"u8.CopyTo(bytes.AsSpan(0, 4));
        bytes[4] = 1;
        bytes[5] = 0;
        bytes[6] = (byte)frame.Kind;
        bytes[7] = (byte)frame.Operation;
        BinaryPrimitives.WriteUInt16BigEndian(bytes.AsSpan(8, 2), frame.Status);
        BinaryPrimitives.WriteUInt16BigEndian(bytes.AsSpan(10, 2), HeaderLength);
        BinaryPrimitives.WriteUInt64BigEndian(bytes.AsSpan(12, 8), frame.RequestId);
        BinaryPrimitives.WriteUInt32BigEndian(bytes.AsSpan(20, 4), (uint)frame.Payload.Length);
        frame.Payload.AsSpan().CopyTo(bytes.AsSpan(HeaderLength));
        return bytes;
    }

    public static WireFrame Decode(ReadOnlySpan<byte> bytes)
    {
        if (bytes.Length < HeaderLength) throw new ArgumentException("Incomplete frame header");
        if (!bytes[..4].SequenceEqual("RSWP"u8)) throw new ArgumentException("Invalid frame magic");
        if (bytes[4] != 1 || bytes[5] != 0) throw new ArgumentException("Unsupported protocol version");

        var kind = (WireKind)bytes[6];
        var operation = (WireOperation)bytes[7];
        ushort status = BinaryPrimitives.ReadUInt16BigEndian(bytes.Slice(8, 2));
        Validate(kind, operation, status);

        if (BinaryPrimitives.ReadUInt16BigEndian(bytes.Slice(10, 2)) != HeaderLength)
            throw new ArgumentException("Invalid frame header length");

        uint bodyLength = BinaryPrimitives.ReadUInt32BigEndian(bytes.Slice(20, 4));
        if (bodyLength > MaxPayloadBytes) throw new ArgumentException("Frame payload too large");
        int expected = HeaderLength + checked((int)bodyLength);
        if (bytes.Length < expected) throw new ArgumentException("Incomplete frame payload");
        if (bytes.Length > expected) throw new ArgumentException("Trailing bytes after frame");

        ulong requestId = BinaryPrimitives.ReadUInt64BigEndian(bytes.Slice(12, 8));
        return new WireFrame(kind, operation, requestId, status, bytes.Slice(HeaderLength).ToArray());
    }

    private static void Validate(WireKind kind, WireOperation operation, ushort status)
    {
        if (kind is not (WireKind.Request or WireKind.Response))
            throw new ArgumentException("Invalid frame kind");
        if (operation is not (WireOperation.Hello or WireOperation.Ping or WireOperation.Put or WireOperation.Get or WireOperation.Delete))
            throw new ArgumentException("Invalid operation");
        if (status > 7 || (kind == WireKind.Request && status != 0))
            throw new ArgumentException("Invalid request/response status");
    }
}
