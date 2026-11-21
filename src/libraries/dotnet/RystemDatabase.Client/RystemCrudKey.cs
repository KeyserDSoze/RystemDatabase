using System.Buffers.Binary;
using System.Text;

namespace RystemDatabase.Client;

/// <summary>
/// Experimental RSWP v1 binary CRUD key body codec, matching Rust and TS.
/// No network authentication, typed RDB document parsing or RPC client.
/// </summary>
public readonly record struct RystemCrudKeyBody(string Collection, string Id, byte[] Document);

public static class RystemCrudKey
{
    private static readonly UTF8Encoding StrictUtf8 = new(false, true);
    private const int MaxPayload = 16 * 1024 * 1024;

    public static byte[] Encode(string collection, string id)
    {
        if (!ValidCollection(collection))
            throw new ArgumentException("Invalid collection name", nameof(collection));
        if (string.IsNullOrEmpty(id) || id.Any(char.IsControl))
            throw new ArgumentException("Invalid ID", nameof(id));
        byte[] idBytes = StrictUtf8.GetBytes(id);
        if (collection.Length + 1 + idBytes.Length > 1024)
            throw new ArgumentException("Invalid ID length", nameof(id));

        byte[] output = new byte[4 + collection.Length + idBytes.Length];
        BinaryPrimitives.WriteUInt16BigEndian(output.AsSpan(0, 2), (ushort)collection.Length);
        Encoding.ASCII.GetBytes(collection.AsSpan(), output.AsSpan(2, collection.Length));
        int idOffset = 2 + collection.Length;
        BinaryPrimitives.WriteUInt16BigEndian(output.AsSpan(idOffset, 2), (ushort)idBytes.Length);
        idBytes.CopyTo(output, idOffset + 2);
        return output;
    }

    public static RystemCrudKeyBody Decode(ReadOnlySpan<byte> bytes)
    {
        if (bytes.Length > MaxPayload) throw new ArgumentException("CRUD body too large");
        if (bytes.Length < 2) throw new ArgumentException("Truncated collection length");
        int collectionLength = BinaryPrimitives.ReadUInt16BigEndian(bytes[..2]);
        if (collectionLength is < 1 or > 128) throw new ArgumentException("Invalid collection length");
        int idOffset = 2 + collectionLength;
        if (bytes.Length < idOffset + 2)
            throw new ArgumentException("Truncated collection or ID length");

        string collection = StrictUtf8.GetString(bytes.Slice(2, collectionLength));
        if (!ValidCollection(collection)) throw new ArgumentException("Invalid collection");
        int idLength = BinaryPrimitives.ReadUInt16BigEndian(bytes.Slice(idOffset, 2));
        if (idLength == 0 || collectionLength + 1 + idLength > 1024)
            throw new ArgumentException("Invalid ID length");
        int end = idOffset + 2 + idLength;
        if (bytes.Length < end) throw new ArgumentException("Truncated ID bytes");

        string id = StrictUtf8.GetString(bytes.Slice(idOffset + 2, idLength));
        if (id.Any(char.IsControl) || id.Length == 0)
            throw new ArgumentException("Invalid ID");
        return new RystemCrudKeyBody(collection, id, bytes[end..].ToArray());
    }

    private static bool ValidCollection(string? name)
    {
        if (name is null || name.Length is < 1 or > 128) return false;
        return name.All(c => c is >= 'a' and <= 'z'
            or >= 'A' and <= 'Z' or >= '0' and <= '9' or '_' or '-');
    }
}
