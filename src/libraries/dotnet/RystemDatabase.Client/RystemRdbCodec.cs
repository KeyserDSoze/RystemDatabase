using System.Buffers.Binary;
using System.Text;

namespace RystemDatabase.Client;

/// <summary>
/// Portable RDB v1 typed document values. Exact integers are long (signed i64),
/// byte arrays are raw binary, and references are a separate native value.
/// An absent object field is different from an explicit Null field.
/// </summary>
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

/// <summary>
/// Portable, bounded, strict big-endian binary RDB v1 codec.
/// This is NOT an authenticated database connection or a JSON codec.
/// </summary>
public static class RystemRdbCodec
{
    private const int MaxBytes = 64 * 1024 * 1024;
    private const int MaxDepth = 64;
    private static readonly UTF8Encoding Utf8 = new(false, true);
    private static ReadOnlySpan<byte> Magic => "RDB\0"u8;

    public static byte[] Encode(RdbValue value)
    {
        var writer = new Writer();
        writer.Write(Magic);
        writer.Byte(1);
        WriteValue(writer, value, 0);
        return writer.Result();
    }

    private sealed class Writer
    {
        private readonly MemoryStream _stream = new();

        private void Guard(int additional)
        {
            if (additional < 0 || _stream.Length + additional > MaxBytes)
                throw new ArgumentException("RDB document too large");
        }
        public void Write(ReadOnlySpan<byte> data)
        {
            Guard(data.Length);
            _stream.Write(data);
        }
        public void Byte(byte value)
        {
            Guard(1);
            _stream.WriteByte(value);
        }
        public void U32(int value)
        {
            if (value < 0) throw new ArgumentException("Invalid RDB length");
            Span<byte> buffer = stackalloc byte[4];
            BinaryPrimitives.WriteUInt32BigEndian(buffer, (uint)value);
            Write(buffer);
        }
        public void I64(long value)
        {
            Span<byte> buffer = stackalloc byte[8];
            BinaryPrimitives.WriteInt64BigEndian(buffer, value);
            Write(buffer);
        }
        public void F64(double value)
        {
            if (!double.IsFinite(value)) throw new ArgumentException("Non-finite RDB float");
            I64(BitConverter.DoubleToInt64Bits(value));
        }
        public void Text(string value)
        {
            ArgumentNullException.ThrowIfNull(value);
            byte[] bytes = Utf8.GetBytes(value);
            U32(bytes.Length);
            Write(bytes);
        }
        public byte[] Result() => _stream.ToArray();
    }

    private static void WriteValue(Writer w, RdbValue value, int depth)
    {
        if (depth > MaxDepth) throw new ArgumentException("RDB nesting too deep");
        if (value is null) throw new ArgumentException("Use RdbValue.Null for explicit null");
        switch (value)
        {
            case RdbValue.Null:
                w.Byte(0);
                break;
            case RdbValue.Boolean v:
                w.Byte(v.Value ? (byte)2 : (byte)1);
                break;
            case RdbValue.Integer v:
                w.Byte(3);
                w.I64(v.Value);
                break;
            case RdbValue.Float v:
                w.Byte(4);
                w.F64(v.Value);
                break;
            case RdbValue.Text v:
                w.Byte(5);
                w.Text(v.Value);
                break;
            case RdbValue.Bytes v:
                ArgumentNullException.ThrowIfNull(v.Value);
                w.Byte(6);
                w.U32(v.Value.Length);
                w.Write(v.Value);
                break;
            case RdbValue.Array v:
                ArgumentNullException.ThrowIfNull(v.Values);
                w.Byte(7);
                w.U32(v.Values.Count);
                foreach (var item in v.Values) WriteValue(w, item, depth + 1);
                break;
            case RdbValue.Object v:
            {
                ArgumentNullException.ThrowIfNull(v.Fields);
                // Rust BTreeMap<String> sorts by UTF-8 byte lexicographic order.
                // .NET ordinal string sorting instead uses UTF-16 code units,
                // which differs for supplementary Unicode code points.
                var sorted = v.Fields
                    .Select(x => (Key: Utf8.GetBytes(x.Key), Value: x.Value))
                    .ToList();
                sorted.Sort((a,b) => a.Key.AsSpan().SequenceCompareTo(b.Key));
                w.Byte(8);
                w.U32(sorted.Count);
                foreach (var entry in sorted)
                {
                    w.U32(entry.Key.Length);
                    w.Write(entry.Key);
                    WriteValue(w, entry.Value, depth + 1);
                }
                break;
            }
            case RdbValue.Reference v:
                if (string.IsNullOrEmpty(v.Collection) || string.IsNullOrEmpty(v.Id))
                    throw new ArgumentException("Invalid RDB reference");
                w.Byte(9);
                w.Text(v.Collection);
                w.Text(v.Id);
                break;
            default:
                throw new ArgumentException("Unknown RDB value type");
        }
    }

    private ref struct Reader
    {
        private readonly ReadOnlySpan<byte> _bytes;
        private int _offset;
        public Reader(ReadOnlySpan<byte> bytes)
        {
            _bytes = bytes;
            _offset = 0;
        }
        public int Remaining => _bytes.Length - _offset;

        public ReadOnlySpan<byte> Take(int count)
        {
            if (count < 0 || count > Remaining) throw new ArgumentException("Truncated RDB document");
            var result = _bytes.Slice(_offset, count);
            _offset += count;
            return result;
        }
        public byte Byte() => Take(1)[0];
        public uint U32() => BinaryPrimitives.ReadUInt32BigEndian(Take(4));
        public long I64() => BinaryPrimitives.ReadInt64BigEndian(Take(8));
        public string Text()
        {
            uint length = U32();
            if (length > (uint)Remaining) throw new ArgumentException("Truncated RDB text");
            return Utf8.GetString(Take((int)length));
        }
    }

    public static RdbValue Decode(ReadOnlySpan<byte> data)
    {
        if (data.Length > MaxBytes) throw new ArgumentException("RDB document too large");
        var r = new Reader(data);
        if (r.Remaining < 5) throw new ArgumentException("Truncated RDB header");
        if (!r.Take(4).SequenceEqual(Magic)) throw new ArgumentException("Invalid RDB magic");
        if (r.Byte() != 1) throw new ArgumentException("Unsupported RDB version");
        var value = ReadValue(ref r, 0);
        if (r.Remaining != 0) throw new ArgumentException("Trailing RDB bytes");
        return value;
    }

    private static RdbValue ReadValue(ref Reader r, int depth)
    {
        if (depth > MaxDepth) throw new ArgumentException("RDB nesting too deep");
        byte tag = r.Byte();
        switch (tag)
        {
            case 0: return new RdbValue.Null();
            case 1: return new RdbValue.Boolean(false);
            case 2: return new RdbValue.Boolean(true);
            case 3: return new RdbValue.Integer(r.I64());
            case 4:
            {
                double value = BitConverter.Int64BitsToDouble(r.I64());
                if (!double.IsFinite(value)) throw new ArgumentException("Non-finite RDB float");
                return new RdbValue.Float(value);
            }
            case 5: return new RdbValue.Text(r.Text());
            case 6:
            {
                uint length = r.U32();
                if (length > (uint)r.Remaining) throw new ArgumentException("Truncated RDB bytes");
                return new RdbValue.Bytes(r.Take((int)length).ToArray());
            }
            case 7:
            {
                uint count = r.U32();
                if (count > (uint)r.Remaining) throw new ArgumentException("Truncated RDB array");
                var items = new List<RdbValue>((int)count);
                for (uint i = 0; i < count; i++) items.Add(ReadValue(ref r, depth + 1));
                return new RdbValue.Array(items);
            }
            case 8:
            {
                uint count = r.U32();
                if (count > (uint)(r.Remaining / 5)) throw new ArgumentException("Truncated RDB object");
                var fields = new Dictionary<string,RdbValue>(StringComparer.Ordinal);
                byte[]? previousKey = null;
                for (uint i=0; i<count; i++)
                {
                    string key = r.Text();
                    var value = ReadValue(ref r, depth + 1);
                    if (fields.ContainsKey(key)) throw new ArgumentException("Duplicate RDB field");
                    // A canonical on-wire object uses UTF-8 byte sorting,
                    // not .NET's ordinal UTF-16 string comparison.
                    byte[] bytes = Utf8.GetBytes(key);
                    if (previousKey is not null &&
                        previousKey.AsSpan().SequenceCompareTo(bytes) >= 0)
                        throw new ArgumentException("Noncanonical RDB object field order");
                    fields.Add(key, value);
                    previousKey = bytes;
                }
                return new RdbValue.Object(fields);
            }
            case 9:
            {
                var collection = r.Text();
                var id = r.Text();
                if (string.IsNullOrEmpty(collection) || string.IsNullOrEmpty(id))
                    throw new ArgumentException("Invalid RDB reference");
                return new RdbValue.Reference(collection,id);
            }
            default: throw new ArgumentException("Invalid RDB type tag");
        }
    }
}
