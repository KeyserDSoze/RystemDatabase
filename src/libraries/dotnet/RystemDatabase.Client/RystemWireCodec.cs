namespace RystemDatabase.Client;

/// <summary>
/// RSWP v1 framing contract. This is not yet a remote database connection.
/// </summary>
public enum WireKind : byte { Request = 1, Response = 2 }
public enum WireOperation : byte { Hello = 1, Ping = 2, Put = 16, Get = 17, Delete = 18 }

public readonly record struct WireFrame(
    WireKind Kind, WireOperation Operation, ulong RequestId, ushort Status, byte[] Payload);

public static class RystemWireCodec
{
    public const int HeaderLength = 24;
    public const int MaxPayloadBytes = 16 * 1024 * 1024;

    public static byte[] Encode(WireFrame frame) =>
        throw new NotSupportedException("RSWP codec not implemented");

    public static WireFrame Decode(ReadOnlySpan<byte> bytes) =>
        throw new NotSupportedException("RSWP codec not implemented");
}
