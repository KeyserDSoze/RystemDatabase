using System.Diagnostics;
using System.Net;
using RystemDatabase.Client;

internal static class LiveContract
{
    public static async Task RunAsync(string binary)
    {
        try
        {
            await RystemLoopbackClient.ConnectAsync(IPAddress.Parse("192.0.2.10"), 8790);
            throw new Exception("Non-loopback RPC connection was allowed");
        }
        catch (ArgumentException) { /* Only localhost allowed until TLS/auth. */ }

        var data = Path.Combine(Path.GetTempPath(),
            $"rystem-dotnet-live-{Guid.NewGuid():N}");
        Directory.CreateDirectory(data);
        try
        {
            await using (var server = await LiveServer.StartAsync(binary, data))
            {
                await using var client = await RystemLoopbackClient.ConnectAsync(
                    IPAddress.Loopback, server.Port);
                await client.PingAsync();
                var doc = Document();
                await client.PutAsync("orders", "🦊", doc);
                await client.PutAsync("orders", "\uFEFFid", doc);

                // The RPC process owns the one on-disk writer for its lifetime.
                // A separate native CLI writer must fail on every supported OS.
                var competing = new ProcessStartInfo(binary)
                {
                    UseShellExecute = false,
                    RedirectStandardOutput = true,
                    RedirectStandardError = true
                };
                competing.ArgumentList.Add("document");
                competing.ArgumentList.Add("delete");
                competing.ArgumentList.Add("--data");
                competing.ArgumentList.Add(data);
                competing.ArgumentList.Add("--collection");
                competing.ArgumentList.Add("orders");
                competing.ArgumentList.Add("--id");
                competing.ArgumentList.Add("🦊");
                using (var contender = Process.Start(competing) ??
                    throw new Exception("Cannot spawn competing native writer"))
                {
                    await contender.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(15));
                    Check(contender.ExitCode != 0,
                        "RPC server writer lock allowed a second process to mutate storage");
                }

                var got = await client.GetAsync("orders", "🦊");
                Check(got is not null &&
                    RystemRdbCodec.Encode(got).AsSpan().SequenceEqual(RystemRdbCodec.Encode(doc)),
                    "Real Rust server returned wrong typed RDB document");
                Check(await client.GetAsync("orders", "missing") is null,
                    "Missing document should return null");

                var tasks = Enumerable.Range(0, 8).Select(async n =>
                {
                    await client.PutAsync("orders", $"n{n}", doc);
                    var response = await client.GetAsync("orders", $"n{n}");
                    Check(response is not null && RystemRdbCodec.Encode(response)
                        .AsSpan().SequenceEqual(RystemRdbCodec.Encode(doc)),
                        "Concurrent requests lost correlation or typed document");
                });
                await Task.WhenAll(tasks);
                Check(await client.DeleteAsync("orders", "🦊"), "First delete should succeed");
                Check(!await client.DeleteAsync("orders", "🦊"), "Second delete should be absent");
                Check(await client.GetAsync("orders", "🦊") is null,
                    "Deleted document was still present");

                try
                {
                    await client.PutAsync("orders", "", doc);
                    throw new Exception("Empty ID accepted");
                }
                catch (ArgumentException) { /* Invalid key must fail client-side. */ }
            }

            // A genuine separate OS process reopens the append-only segment log.
            await using (var restarted = await LiveServer.StartAsync(binary, data))
            await using (var client = await RystemLoopbackClient.ConnectAsync(
                IPAddress.Loopback, restarted.Port))
            {
                await client.PingAsync();
                var persisted = await client.GetAsync("orders", "\uFEFFid");
                Check(persisted is not null && RystemRdbCodec.Encode(persisted)
                    .AsSpan().SequenceEqual(RystemRdbCodec.Encode(Document())),
                    "Cross-process server restart lost native typed document");
                Check(await client.DeleteAsync("orders", "\uFEFFid"),
                    "Persistent native record not deletable");
            }
            Console.WriteLine("Rystem .NET LIVE loopback client + Rust TCP server: PASS");
        }
        finally
        {
            if (Directory.Exists(data)) Directory.Delete(data, recursive: true);
        }
    }

    private static RdbValue.Object Document() => new(
        new Dictionary<string, RdbValue>
        {
            ["array"] = new RdbValue.Array([
                new RdbValue.Null(), new RdbValue.Integer(long.MinValue),
                new RdbValue.Boolean(true)
            ]),
            ["binary"] = new RdbValue.Bytes([0, 255, 128]),
            ["reference"] = new RdbValue.Reference("users", "u42"),
            ["unicode"] = new RdbValue.Text("Roma 🏛")
        });

    private static void Check(bool ok, string msg)
    {
        if (!ok) throw new Exception(msg);
    }

    private sealed class LiveServer : IAsyncDisposable
    {
        private readonly Process _process;
        public int Port { get; }

        private LiveServer(Process process, int port)
        {
            _process = process;
            Port = port;
        }
        public static async Task<LiveServer> StartAsync(string binary, string data)
        {
            var start = new ProcessStartInfo(binary)
            {
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true
            };
            start.ArgumentList.Add("rpc-serve");
            start.ArgumentList.Add("--data");
            start.ArgumentList.Add(data);
            start.ArgumentList.Add("--listen");
            start.ArgumentList.Add("127.0.0.1:0");
            var process = Process.Start(start) ??
                throw new Exception("Unable to spawn native Rust Rystem server");
            try
            {
                var banner = await process.StandardOutput.ReadLineAsync()
                    .WaitAsync(TimeSpan.FromSeconds(20));
                if (banner is null || !banner.Contains("LOOPBACK RPC listening on "))
                    throw new Exception($"Rust RPC server did not report bound loopback port: {banner}");
                var address = IPEndPoint.Parse(banner.Split(' ').Last());
                if (!IPAddress.IsLoopback(address.Address))
                    throw new Exception("Rust server reported non-loopback binding");
                return new LiveServer(process, address.Port);
            }
            catch
            {
                if (!process.HasExited) process.Kill(entireProcessTree: true);
                await process.WaitForExitAsync();
                process.Dispose();
                throw;
            }
        }
        public async ValueTask DisposeAsync()
        {
            if (!_process.HasExited) _process.Kill(entireProcessTree: true);
            await _process.WaitForExitAsync();
            _process.Dispose();
        }
    }
}
