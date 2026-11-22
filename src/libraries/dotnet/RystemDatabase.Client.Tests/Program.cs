using RystemDatabase.Client;

static void Check(bool ok, string message)
{
    if (!ok) throw new Exception(message);
}

var ping = new WireFrame(WireKind.Request, WireOperation.Ping, 7, 0, Array.Empty<byte>());
var golden = new byte[] {
    0x52,0x53,0x57,0x50,1,0,1,2,0,0,0,24,
    0,0,0,0,0,0,0,7,0,0,0,0
};
var bytes = RystemWireCodec.Encode(ping);
Check(bytes.AsSpan().SequenceEqual(golden), "RSWP Rust/.NET PING golden vector differs");
var fromGolden = RystemWireCodec.Decode(golden);
Check(fromGolden.RequestId == 7 && fromGolden.Kind == WireKind.Request &&
      fromGolden.Operation == WireOperation.Ping && fromGolden.Status == 0 &&
      fromGolden.Payload.Length == 0, "Cannot decode known Rust PING frame");
var payload = new byte[] { 0, 255, 128 };
var bigId = new WireFrame(WireKind.Response, WireOperation.Get, ulong.MaxValue, 1, payload);
var roundTrip = RystemWireCodec.Decode(RystemWireCodec.Encode(bigId));
Check(roundTrip.RequestId == ulong.MaxValue && roundTrip.Payload.AsSpan().SequenceEqual(payload),
      "Correlation id or binary payload lost");
for (int i = 0; i < golden.Length; i++)
{
    try
    {
        RystemWireCodec.Decode(golden.AsSpan(0, i));
        throw new Exception($"Truncated frame at {i} accepted");
    }
    catch (ArgumentException)
    {
        // Expected: fail closed on an incomplete frame.
    }
}
Console.WriteLine("Rystem .NET wire protocol conformance: PASS");


// RED: independent .NET CRUD body compatibility tests against Rust wire literals.
var expectedKey = new byte[] { 0,5,117,115,101,114,115,0,3,117,52,50 };
Check(RystemCrudKey.Encode("users","u42").AsSpan().SequenceEqual(expectedKey),
    "C# CRUD key must be byte-identical to Rust/TypeScript golden vector");
var parsed = RystemCrudKey.Decode(expectedKey);
Check(parsed.Collection == "users" && parsed.Id == "u42" && parsed.Document.Length == 0,
    "C# decode of known GET body failed");

var unicodeKey = new byte[] { 0,6,111,114,100,101,114,115,0,4,240,159,166,138 };
Check(RystemCrudKey.Encode("orders","🦊").AsSpan().SequenceEqual(unicodeKey),
    "C# UTF-8 ID must use byte length not UTF-16 length");
var documentBytes = new byte[] {
    82,68,66,0,1,8,0,0,0,1,0,0,0,1,110,3,0,0,0,0,0,0,0,42
};
var put = unicodeKey.Concat(documentBytes).ToArray();
var putDecoded = RystemCrudKey.Decode(put);
Check(putDecoded.Id == "🦊" && putDecoded.Collection == "orders" &&
    putDecoded.Document.AsSpan().SequenceEqual(documentBytes),
    "C# decode must preserve raw typed binary document bytes");

foreach (var invalidCollection in new[] { "", "bad/name", "a b", "é", new string('x',129) })
{
    try { RystemCrudKey.Encode(invalidCollection, "id"); throw new Exception("Invalid collection accepted"); }
    catch (ArgumentException) { /* expected */ }
}
foreach (var invalidId in new[] { "", "\n", "\u0085", new string('x',1024) })
{
    try { RystemCrudKey.Encode("users", invalidId); throw new Exception("Invalid ID accepted"); }
    catch (ArgumentException) { /* expected */ }
}
for (int n = 0; n < expectedKey.Length; n++)
{
    try { RystemCrudKey.Decode(expectedKey.AsSpan(0,n)); throw new Exception("Incomplete key accepted"); }
    catch (ArgumentException) { /* expected */ }
}
var invalidUtf8 = expectedKey.ToArray();
invalidUtf8[10] = 255;
try { RystemCrudKey.Decode(invalidUtf8); throw new Exception("Invalid UTF-8 accepted"); }
catch (ArgumentException) { /* expected */ }
Console.WriteLine("Rystem .NET CRUD key conformance: PASS");


// RED: typed value bytes must match Rust/TypeScript exactly, not JSON.
var nestedDoc = new RdbValue.Object(new Dictionary<string,RdbValue>(StringComparer.Ordinal)
{
    ["a"] = new RdbValue.Array(new RdbValue[] {
        new RdbValue.Null(), new RdbValue.Boolean(true), new RdbValue.Integer(-1)
    }),
    ["b"] = new RdbValue.Bytes(new byte[]{0,255,128}),
    ["r"] = new RdbValue.Reference("users","u42")
});
var goldenTyped = Convert.FromHexString(
    "52444200010800000003" +
    "00000001610700000003000203ffffffffffffffff" +
    "0000000162060000000300ff80" +
    "00000001720900000005757365727300000003753432");
Check(RystemRdbCodec.Encode(nestedDoc).AsSpan().SequenceEqual(goldenTyped),
    "C# nested typed RDB golden differs from Rust/TS");
var decodedTyped = RystemRdbCodec.Decode(goldenTyped);
Check(decodedTyped is RdbValue.Object, "Expected typed object");
Check(RystemRdbCodec.Encode(decodedTyped).AsSpan().SequenceEqual(goldenTyped),
    "Decoded nested RDB object did not survive roundtrip");
var floatDoc = new RdbValue.Object(new Dictionary<string,RdbValue>
{
    ["absent-versus-null"] = new RdbValue.Null(),
    ["min"] = new RdbValue.Integer(long.MinValue),
    ["max"] = new RdbValue.Integer(long.MaxValue),
    ["float"] = new RdbValue.Float(1.25),
    ["name"] = new RdbValue.Text("Roma 🏛")
});
Check(RystemRdbCodec.Encode(RystemRdbCodec.Decode(RystemRdbCodec.Encode(floatDoc)))
    .AsSpan().SequenceEqual(RystemRdbCodec.Encode(floatDoc)), "C# typed roundtrip failed");
try { RystemRdbCodec.Encode(new RdbValue.Float(double.NaN)); throw new Exception("NaN accepted"); }
catch (ArgumentException) { /* expected */ }
try { RystemRdbCodec.Encode(new RdbValue.Reference("","id")); throw new Exception("Empty reference accepted"); }
catch (ArgumentException) { /* expected */ }
for(int n=0;n<goldenTyped.Length;n++)
{
    try { RystemRdbCodec.Decode(goldenTyped.AsSpan(0,n)); throw new Exception($"Truncated RDB accepted at {n}"); }
    catch (ArgumentException) { /* expected */ }
}
try { RystemRdbCodec.Decode(goldenTyped.Concat(new byte[]{0}).ToArray()); throw new Exception("Trailing bytes accepted"); }
catch (ArgumentException) { /* expected */ }
var unknownTag = new byte[]{82,68,66,0,1,255};
try { RystemRdbCodec.Decode(unknownTag); throw new Exception("Unknown type tag accepted"); }
catch (ArgumentException) { /* expected */ }
var duplicateFields = Convert.FromHexString("52444200010800000002000000017800000000017802");
try { RystemRdbCodec.Decode(duplicateFields); throw new Exception("Duplicate keys accepted"); }
catch (ArgumentException) { /* expected */ }
Console.WriteLine("Rystem .NET typed RDB v1 conformance: PASS");

// Leading U+FEFF is a Unicode scalar in RSWP/RDB data, not a byte-order
// signature to discard from individual decoded UTF-8 string slices.
var bomId = "\uFEFFid";
Check(RystemCrudKey.Decode(RystemCrudKey.Encode("docs",bomId)).Id == bomId,
    "C# CRUD key must preserve U+FEFF in IDs");
var bomTyped = new RdbValue.Object(new Dictionary<string,RdbValue>
{
    ["\uFEFFname"] = new RdbValue.Text("\uFEFFvalue"),
    ["reference"] = new RdbValue.Reference("users","\uFEFFid")
});
Check(RystemRdbCodec.Encode(RystemRdbCodec.Decode(RystemRdbCodec.Encode(bomTyped)))
    .AsSpan().SequenceEqual(RystemRdbCodec.Encode(bomTyped)),
    "C# typed codec must preserve UTF-8 U+FEFF scalar in data");

if (args.Length > 0)
{
    await LiveContract.RunAsync(args[0]);
    await TlsContract.RunAsync(args[0]);
}


// #65 RED: the Rust/TypeScript/.NET object-key codec contract requires
// strictly increasing UTF-8 key bytes, also on decode. Distinct reversed
// keys must not be silently re-encoded to a different canonical byte string.
foreach (var invalidObject in new[] {
    "52444200010800000002000000016200000000016102",
    "5244420001080000000200000004f09f98800000000003ee808002",
    "52444200010800000001000000056f757465720800000002000000016200000000016102",
}) {
    try {
        RystemRdbCodec.Decode(Convert.FromHexString(invalidObject));
        throw new Exception("Noncanonical RDB object key order was accepted");
    } catch (ArgumentException) {
        // Expected only after strict canonical ordering is implemented.
    }
}
for (byte larger = (byte)'b'; larger <= (byte)'z'; larger++) {
    for (byte smaller = (byte)'a'; smaller < larger; smaller++) {
        var descending = new byte[] {82,68,66,0,1,8,0,0,0,2,
            0,0,0,1,larger,0,0,0,0,1,smaller,2};
        try {
            RystemRdbCodec.Decode(descending);
            throw new Exception($"Descending RDB object key order accepted: {larger} > {smaller}");
        } catch (ArgumentException) {
            // Expected.
        }
    }
}
Console.WriteLine("Rystem .NET strict canonical RDB object keys: PASS");
