namespace RystemDatabase.Client;

/// <summary>Binary CRUD key body codec contract. RED/TDD until implemented.</summary>
public readonly record struct RystemCrudKeyBody(string Collection, string Id, byte[] Document);
public static class RystemCrudKey
{
    public static byte[] Encode(string collection, string id)
        => throw new NotSupportedException("CRUD key codec missing");

    public static RystemCrudKeyBody Decode(ReadOnlySpan<byte> bytes)
        => throw new NotSupportedException("CRUD key codec missing");
}
