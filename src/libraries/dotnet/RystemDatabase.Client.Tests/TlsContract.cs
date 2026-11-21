using System.Diagnostics;
using System.Net;
using System.Security.Authentication;
using RystemDatabase.Client;

/// <summary>Real OS-process TLS1.3 interoperability, no fake sockets/cert bypass.</summary>
internal static class TlsContract
{
    private static readonly byte[] ReadToken = Enumerable.Repeat((byte)0x11, 32).ToArray();
    private static readonly byte[] WriteToken = Enumerable.Repeat((byte)0x22, 32).ToArray();

    public static async Task RunAsync(string binary)
    {
        // These fixtures are PUBLIC, deliberately untrusted and for testing only.
        var fixtures = Path.GetFullPath(Path.Combine("src", "rystemdb-cli", "tests", "fixtures", "tls"));
        var root = Path.Combine(Path.GetTempPath(), "rystem-net-tls-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try
        {
            var auth = Path.Combine(root, "acl.txt");
            var cert = Path.Combine(root, "server-cert.pem");
            var key = Path.Combine(root, "server-key.pem");
            var store = Path.Combine(root, "store");
            var ca = Path.Combine(fixtures, "root-ca.pem");
            var wrongCa = Path.Combine(fixtures, "wrong-root-ca.pem");
            await File.WriteAllTextAsync(auth,
                new string('1', 64) + " orders r\n" +
                new string('2', 64) + " orders w\n");
            File.Copy(Path.Combine(fixtures, "server-cert.pem"), cert);
            File.Copy(Path.Combine(fixtures, "server-key.pem"), key);
            if (!OperatingSystem.IsWindows())
            {
                File.SetUnixFileMode(auth, UnixFileMode.UserRead | UnixFileMode.UserWrite);
                File.SetUnixFileMode(key, UnixFileMode.UserRead | UnixFileMode.UserWrite);
            }

            // Validation must reject a nonlocal target before opening any TCP socket.
            try
            {
                await RystemLoopbackClient.ConnectTlsAsync(IPAddress.Parse("192.0.2.9"), 8790,
                    "localhost", ca, ReadToken);
                throw new Exception("TLS client allowed a non-loopback address");
            }
            catch (ArgumentException) { }

            await using (var server = await RealTlsServer.StartAsync(binary, store, auth, cert, key))
            {
                await RejectAuthenticationAsync(
                    () => RystemLoopbackClient.ConnectTlsAsync(IPAddress.Loopback, server.Port,
                        "not-localhost.test", ca, ReadToken),
                    "wrong server hostname");
                await RejectAuthenticationAsync(
                    () => RystemLoopbackClient.ConnectTlsAsync(IPAddress.Loopback, server.Port,
                        "localhost", wrongCa, ReadToken),
                    "untrusted CA");
                // Repeated real TLS handshakes catch the macOS
                // Network.framework race where a server closed TCP before
                // the client's 24-byte status 6 RSWP header was delivered.
                // The server now flushes the frame and sends TLS close_notify.
                for (var attempt = 0; attempt < 12; attempt++)
                {
                    try
                    {
                        await RystemLoopbackClient.ConnectTlsAsync(IPAddress.Loopback, server.Port,
                            "localhost", ca, new byte[32]);
                        throw new Exception("Invalid bearer token was accepted after TLS handshake");
                    }
                    catch (RystemRpcException e) when (e.Status == 6) { }
                }

                // Existing plaintext sockets must NOT downgrade a TLS endpoint.
                await RejectTransportAsync(
                    () => RystemLoopbackClient.ConnectWithTokenAsync(
                        IPAddress.Loopback, server.Port, WriteToken),
                    "plaintext-to-TLS downgrade");

                await using var writer = await RystemLoopbackClient.ConnectTlsAsync(
                    IPAddress.Loopback, server.Port, "localhost", ca, WriteToken);
                await using var reader = await RystemLoopbackClient.ConnectTlsAsync(
                    IPAddress.Loopback, server.Port, "localhost", ca, ReadToken);
                await writer.PingAsync();
                var doc = Document();
                await writer.PutAsync("orders", "\uFEFFid", doc);
                await writer.PutAsync("orders", "🦊", doc);
                var found = await reader.GetAsync("orders", "🦊");
                Check(found is not null && RystemRdbCodec.Encode(found)
                    .AsSpan().SequenceEqual(RystemRdbCodec.Encode(doc)),
                    "Native Rust TLS server changed typed bytes/references");
                await UnauthorizedAsync(() => writer.GetAsync("orders", "🦊"),
                    "write-only TLS token read protected document");
                await UnauthorizedAsync(() => reader.DeleteAsync("orders", "🦊"),
                    "read-only TLS token deleted protected document");
                await UnauthorizedAsync(() => writer.PutAsync("other", "bad", doc),
                    "TLS token wrote unknown collection");
                await Task.WhenAll(Enumerable.Range(0, 6).Select(async i =>
                {
                    await writer.PutAsync("orders", "n" + i, doc);
                    var value = await reader.GetAsync("orders", "n" + i);
                    Check(value is not null, "TLS concurrent binary GET failed");
                }));
                Check(await writer.DeleteAsync("orders", "🦊"), "TLS delete returned false");
                Check(await reader.GetAsync("orders", "🦊") is null, "TLS delete resurrected");
            }
            // True OS process restart, not just a new TCP socket.
            await using (var restarted = await RealTlsServer.StartAsync(binary, store, auth, cert, key))
            await using (var reader = await RystemLoopbackClient.ConnectTlsAsync(
                IPAddress.Loopback, restarted.Port, "localhost", ca, ReadToken))
            {
                var persisted = await reader.GetAsync("orders", "\uFEFFid");
                Check(persisted is not null &&
                    RystemRdbCodec.Encode(persisted).AsSpan().SequenceEqual(RystemRdbCodec.Encode(Document())),
                    "TLS restart lost persisted binary data");
            }
            // The chain and SAN are valid, but leaf validity dates are not:
            // .NET must reject both EXPIRED and NOT-YET-VALID certificates.
            var validityRoot = Path.Combine(fixtures, "validity-root-ca.pem");
            var validityKey = Path.Combine(root, "validity-server-key.pem");
            File.Copy(Path.Combine(fixtures, "validity-server-key.pem"), validityKey);
            if (!OperatingSystem.IsWindows())
                File.SetUnixFileMode(validityKey, UnixFileMode.UserRead | UnixFileMode.UserWrite);
            foreach (var kind in new[] { "expired", "future" })
            {
                var wrongDateCert = Path.Combine(root, kind + "-server-cert.pem");
                File.Copy(Path.Combine(fixtures, kind + "-server-cert.pem"), wrongDateCert);
                await using var invalidServer = await RealTlsServer.StartAsync(
                    binary, Path.Combine(root, kind + "-store"), auth, wrongDateCert, validityKey);
                await RejectAuthenticationAsync(() => RystemLoopbackClient.ConnectTlsAsync(
                    IPAddress.Loopback, invalidServer.Port, "localhost", validityRoot, ReadToken),
                    kind + " server certificate");
            }
            Console.WriteLine("Rystem .NET verified TLS1.3/native-Rust TCP integration: PASS");
        }
        finally
        {
            if (Directory.Exists(root)) Directory.Delete(root, recursive:true);
        }
    }

    private static async Task RejectAuthenticationAsync(Func<Task<RystemLoopbackClient>> action, string reason)
    {
        try
        {
            await using var unexpected = await action();
            throw new Exception("TLS accepted " + reason);
        }
        catch (AuthenticationException) { }
    }
    private static async Task RejectTransportAsync(Func<Task<RystemLoopbackClient>> action, string reason)
    {
        try
        {
            await using var unexpected = await action();
            throw new Exception("TLS accepted " + reason);
        }
        catch (Exception e) when (e is IOException or AuthenticationException) { }
    }
    private static async Task UnauthorizedAsync(Func<Task> action, string reason)
    {
        try { await action(); throw new Exception(reason); }
        catch (RystemRpcException e) when (e.Status == 6) { }
    }
    private static RdbValue.Object Document() => new(new Dictionary<string, RdbValue>
    {
        ["bytes"] = new RdbValue.Bytes([0,255,128]),
        ["reference"] = new RdbValue.Reference("users", "u42"),
        ["array"] = new RdbValue.Array([new RdbValue.Integer(long.MinValue),
            new RdbValue.Null(), new RdbValue.Boolean(true)])
    });
    private static void Check(bool ok, string reason)
    {
        if (!ok) throw new Exception(reason);
    }
    private sealed class RealTlsServer : IAsyncDisposable
    {
        private readonly Process _process;
        public int Port { get; }
        private RealTlsServer(Process process, int port)
        {
            _process = process;
            Port = port;
        }

        public static async Task<RealTlsServer> StartAsync(string binary, string store,
            string acl, string cert, string key)
        {
            var info = new ProcessStartInfo(binary)
            {
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true
            };
            foreach (var arg in new [] {"rpc-serve", "--data", store, "--listen", "127.0.0.1:0",
                "--auth-file", acl, "--tls-cert", cert, "--tls-key", key})
                info.ArgumentList.Add(arg);
            var process = Process.Start(info) ?? throw new Exception("Could not start native Rust TLS server");
            try
            {
                string? banner = await process.StandardOutput.ReadLineAsync()
                    .WaitAsync(TimeSpan.FromSeconds(20));
                if (banner is null || !banner.Contains("LOOPBACK RPC listening on "))
                    throw new Exception("No native TLS readiness banner");
                var endpoint = IPEndPoint.Parse(banner.Split(' ').Last());
                if (!IPAddress.IsLoopback(endpoint.Address))
                    throw new Exception("Rust TLS server escaped loopback");
                return new RealTlsServer(process, endpoint.Port);
            }
            catch
            {
                if (!process.HasExited) process.Kill(entireProcessTree:true);
                await process.WaitForExitAsync();
                process.Dispose();
                throw;
            }
        }
        public async ValueTask DisposeAsync()
        {
            if (!_process.HasExited) _process.Kill(entireProcessTree:true);
            await _process.WaitForExitAsync();
            _process.Dispose();
        }
    }
}
