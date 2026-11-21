namespace RystemDatabase.Client;

/// <summary>RDB v1 native binary value model, not a database connection.</summary>
public abstract record RdbValue
{
    public sealed record Null : RdbValue;
    public sealed record Boolean(bool Value) : RdbValue;
    public sealed record Integer(long Value) : RdbValue;
    public sealed record Float(double Value) : RdbValue;
    public sealed record Text(string Value) : RdbValue;
    public sealed record Bytes(byte[] Value) : RdbValue;
    public sealed record Array(IReadOnlyList<RdbValue> Values) : RdbValue;
    public sealed record Object(IReadOnlyDictionary<string,RdbValue> Fields) : RdbValue;
    public sealed record Reference(string Collection, string Id) : RdbValue;
}

public static class RystemRdbCodec
{
    public static byte[] Encode(RdbValue value)
        => throw new NotSupportedException("RDB v1 typed codec not implemented");
    public static RdbValue Decode(ReadOnlySpan<byte> data)
        => throw new NotSupportedException("RDB v1 typed codec not implemented");
}
