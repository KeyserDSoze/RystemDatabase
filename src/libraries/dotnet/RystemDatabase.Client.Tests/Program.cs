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
