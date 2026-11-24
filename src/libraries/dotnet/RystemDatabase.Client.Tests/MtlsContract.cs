using System.Diagnostics;
using System.Net;
using System.Security.Authentication;
using RystemDatabase.Client;

/// <summary>
/// #69 RED: actual native Rust TLS1.3 mTLS with ephemeral, CA-signed client
/// identities and principal-bound collection grants. Never commit client keys.
/// </summary>
internal static class MtlsContract
{
    private static readonly byte[] WriterToken = Enumerable.Repeat((byte)0x11, 32).ToArray();
    private static readonly byte[] WrongToken = Enumerable.Repeat((byte)0x22, 32).ToArray();

    private sealed record Identity(string Certificate, string PrivateKey);

    // #99 RED: validate the repeat configuration without starting a server.
    private static void VerifyDenialRoundConfiguration()
    {
        Require(ParseDenialRounds(null) == 1, "Default mTLS rounds must be one");
        for (int count = 1; count <= 20; count++)
            Require(ParseDenialRounds(count.ToString(System.Globalization.CultureInfo.InvariantCulture))
                == count, "mTLS round count changed");
        foreach (string invalid in new[] { "", "0", "21", "-1", "+1", "01",
            " 1", "1 ", "1.0", "١", "99999999999999999999" })
        {
            try
            {
                _ = ParseDenialRounds(invalid);
                throw new InvalidOperationException("Invalid mTLS round count accepted");
            }
            catch (ArgumentException) { }
        }
    }

    private static int ParseDenialRounds(string? configured)
    {
        if (configured is null) return 1;
        if (configured.Length is < 1 or > 2 || configured[0] == '0' ||
            configured.Any(c => c is < '0' or > '9') ||
            !int.TryParse(configured, System.Globalization.NumberStyles.None,
                System.Globalization.CultureInfo.InvariantCulture, out int count) ||
            count is < 1 or > 20)
            throw new ArgumentException("RYSTEMDB_MTLS_DENIAL_ROUNDS must be an integer from 1 to 20");
        return count;
    }

    // #98: deterministic negative-contract checks, independent of TLS/platform.
    public static async Task RunDenialAssertionsAsync()
    {
        VerifyDenialRoundConfiguration();
        static Func<Task<RystemLoopbackClient>> Fault(Exception error)
            => () => Task.FromException<RystemLoopbackClient>(error);

        static async Task MustPropagateAsync(Exception error, bool requireHelloStatus)
        {
            try
            {
                await DeniedConnectAsync(Fault(error), "synthetic rejection",
                    requireHelloStatus);
            }
            catch (Exception observed) when (ReferenceEquals(observed, error))
            {
                return;
            }
            throw new Exception("Negative mTLS assertion accepted an unrelated failure");
        }

        await DeniedConnectAsync(Fault(new RystemRpcException(6)),
            "synthetic explicit Unauthorized", requireHelloStatus: true);
        await DeniedConnectAsync(Fault(new AuthenticationException()),
            "synthetic TLS rejection");
        foreach (bool requireHelloStatus in new[] { false, true })
        {
            await MustPropagateAsync(new RystemRpcException(5), requireHelloStatus);
            await MustPropagateAsync(new OperationCanceledException(), requireHelloStatus);
            await MustPropagateAsync(new InvalidDataException(), requireHelloStatus);
            try
            {
                // A null stand-in avoids constructing a real network client;
                // the successful task must still be rejected by the assertion.
                await DeniedConnectAsync(() => Task.FromResult<RystemLoopbackClient>(null!),
                    "synthetic unexpected success", requireHelloStatus);
            }
            catch (InvalidOperationException)
            {
                continue;
            }
            throw new Exception("Negative mTLS assertion accepted successful connection");
        }
        await MustPropagateAsync(new IOException(), true);
        await MustPropagateAsync(new AuthenticationException(), true);
        await MustPropagateAsync(new System.Net.Sockets.SocketException(
            (int)System.Net.Sockets.SocketError.ConnectionRefused), true);
        Console.WriteLine("Rystem .NET mTLS denial assertion contract: PASS");
    }

    public static async Task RunAsync(string binary)
    {
        int rounds = ParseDenialRounds(Environment.GetEnvironmentVariable("RYSTEMDB_MTLS_DENIAL_ROUNDS"));
        using var test = new Fixture();
        await using var server = await test.StartAsync(binary);
        var root = test.ServerRoot;
        var writer = test.Writer;

        // The real .NET SslStream must present the offered signed client
        // identity; never infer authorization from the 32-byte token alone.
        async Task<RystemLoopbackClient> Connect(Identity identity, byte[]? token = null,
            string serverName = "localhost", string? ca = null)
            => await RystemLoopbackClient.ConnectMtlsAsync(IPAddress.Loopback, server.Port,
                serverName, ca ?? root, identity.Certificate, identity.PrivateKey,
                token ?? WriterToken);

        // #99: each round makes fresh connections. Stop on the first
        // unexpected outcome; repeating the workload is not a retry.
        for (int round = 1; round <= rounds; round++)
        {
            Console.WriteLine($"mTLS denial round {round}/{rounds}: START");
            // Missing client certificate must never authenticate just with a token.
            await DeniedConnectAsync(() => RystemLoopbackClient.ConnectTlsAsync(
                IPAddress.Loopback, server.Port, "localhost", root, WriterToken),
                "mTLS server accepted bearer token alone");
            await DeniedConnectAsync(() => Connect(test.WrongIssuer), "untrusted client issuer");
            await DeniedConnectAsync(() => Connect(test.WrongEku), "wrong clientAuth EKU");
            await DeniedConnectAsync(() => Connect(test.Stranger), "unknown verified URI SAN principal");
            await DeniedConnectAsync(() => Connect(writer, WrongToken),
                "disjoint bearer token/identity grants", requireHelloStatus: true);
            await DeniedConnectAsync(() => Connect(writer, new byte[32]),
                "invalid bearer token", requireHelloStatus: true);
            await DeniedConnectAsync(() => Connect(writer, serverName:"not-localhost.test"),
                "incorrect server SAN hostname");
            await DeniedConnectAsync(() => Connect(writer, ca:test.WrongServerRoot),
                "untrusted server issuer");
            await using (var healthy = await Connect(writer))
                await healthy.PingAsync();
            Console.WriteLine($"mTLS denial round {round}/{rounds}: PASS");
        }

        // #69: malformed/bad-pair client credentials must be rejected before
        // any TCP connect, so use a deliberately closed port 1 for these.
        await ExpectBadPemAsync(() => RystemLoopbackClient.ConnectMtlsAsync(
            IPAddress.Loopback, 1, "localhost", root, writer.Certificate, "", WriterToken),
            "missing client private key path");
        await ExpectBadPemAsync(() => RystemLoopbackClient.ConnectMtlsAsync(
            IPAddress.Loopback, 1, "localhost", root, writer.Certificate,
            test.Reader.PrivateKey, WriterToken), "mismatched client certificate/key");
        var malformedCertificate = Path.Combine(test.DirectoryPath, "malformed-client.pem");
        File.WriteAllText(malformedCertificate, "NOT A PEM CERTIFICATE");
        await ExpectBadPemAsync(() => RystemLoopbackClient.ConnectMtlsAsync(
            IPAddress.Loopback, 1, "localhost", root, malformedCertificate,
            writer.PrivateKey, WriterToken), "malformed client certificate");
        var hugeCertificate = Path.Combine(test.DirectoryPath, "oversized-client.pem");
        File.WriteAllText(hugeCertificate, new string('x', 65537));
        await ExpectBadPemAsync(() => RystemLoopbackClient.ConnectMtlsAsync(
            IPAddress.Loopback, 1, "localhost", root, hugeCertificate,
            writer.PrivateKey, WriterToken), "oversized client certificate");
        if (!OperatingSystem.IsWindows())
        {
            var looseKey = Path.Combine(test.DirectoryPath, "loose-client.key");
            File.Copy(writer.PrivateKey, looseKey);
            File.SetUnixFileMode(looseKey, UnixFileMode.UserRead |
                UnixFileMode.UserWrite | UnixFileMode.GroupRead);
            await ExpectBadPemAsync(() => RystemLoopbackClient.ConnectMtlsAsync(
                IPAddress.Loopback, 1, "localhost", root, writer.Certificate,
                looseKey, WriterToken), "client key group readable");
            var linkedKey = Path.Combine(test.DirectoryPath, "symlink-client.key");
            File.CreateSymbolicLink(linkedKey, writer.PrivateKey);
            await ExpectBadPemAsync(() => RystemLoopbackClient.ConnectMtlsAsync(
                IPAddress.Loopback, 1, "localhost", root, writer.Certificate,
                linkedKey, WriterToken), "client private key symlink");
        }
        try
        {
            await RystemLoopbackClient.ConnectMtlsAsync(IPAddress.Parse("192.0.2.9"),
                server.Port, "localhost", root, writer.Certificate,
                writer.PrivateKey, WriterToken);
            throw new Exception("mTLS client permitted a non-loopback target");
        }
        catch (ArgumentException) { }

        await using var admin = await Connect(writer);
        await using var reader = await Connect(test.Reader);
        await admin.PingAsync();
        var document = new RdbValue.Object(new Dictionary<string,RdbValue>
        {
            ["i64"] = new RdbValue.Integer(long.MinValue),
            ["bytes"] = new RdbValue.Bytes([0, 255, 128]),
            ["ref"] = new RdbValue.Reference("users", "u42"),
            ["items"] = new RdbValue.Array([new RdbValue.Null(), new RdbValue.Boolean(true)])
        });
        await admin.PutAsync("orders", "secure", document);
        var found = await reader.GetAsync("orders", "secure");
        Require(found is not null &&
            RystemRdbCodec.Encode(found).AsSpan().SequenceEqual(RystemRdbCodec.Encode(document)),
            "mTLS reader received different typed document bytes");
        await DeniedOperationAsync(() => reader.PutAsync("orders", "denied", document),
            "read-only certificate modified collection");
        await DeniedOperationAsync(() => reader.DeleteAsync("orders", "secure"),
            "read-only certificate deleted document");
        await DeniedOperationAsync(() => admin.GetAsync("inventory", "hidden"),
            "bearer token accessed collection not permitted by certificate");
        Require(await admin.DeleteAsync("orders", "secure"), "writer delete failed");
        Require(await reader.GetAsync("orders", "secure") is null, "deleted record reappeared");
        Console.WriteLine("Rystem .NET verified client-mTLS Rust principal/token ACL: PASS");
    }

    private static void Require(bool ok, string message)
    {
        if (!ok) throw new Exception(message);
    }

    private static async Task DeniedConnectAsync(
        Func<Task<RystemLoopbackClient>> connect, string reason, bool requireHelloStatus = false)
    {
        var elapsed = Stopwatch.StartNew();
        Console.WriteLine("mTLS denial case: " + reason);
        try
        {
            await using var unexpected = await connect();
            throw new InvalidOperationException("mTLS accepted " + reason);
        }
        catch (Exception ex)
        {
            // A verified principal with a bad/disjoint token must receive
            // the explicit RSWP Unauthorized HELLO status, not merely EOF.
            // Other certificate denial cases can fail during TLS or before
            // HELLO. Cancellation and malformed protocol data never count.
            bool rejected = ex is RystemRpcException { Status: 6 } ||
                (!requireHelloStatus && ex is not InvalidDataException &&
                    (ex is AuthenticationException or IOException or
                        System.Net.Sockets.SocketException));
            string outcome = ex is RystemRpcException rpc
                ? $"RPC status {rpc.Status}" : ex.GetType().Name;
            Console.WriteLine($"mTLS denial case: {reason}; {outcome}; " +
                $"{elapsed.ElapsedMilliseconds} ms; accepted rejection={rejected}");
            // Do not log exception messages, credentials or certificate paths.
            // Preserve the original failure and stack for CI investigation.
            if (!rejected) throw;
        }
    }

    private static async Task ExpectBadPemAsync(
        Func<Task<RystemLoopbackClient>> action, string reason)
    {
        try
        {
            await using var unexpected = await action();
            throw new Exception("mTLS accepted invalid client PEM: " + reason);
        }
        catch (ArgumentException) { }
        // Socket errors are deliberately NOT accepted: validation must run
        // before the deliberately unreachable port is ever contacted.
    }

    private static async Task DeniedOperationAsync(Func<Task> op, string reason)
    {
        try { await op(); throw new Exception(reason); }
        catch (RystemRpcException error) when (error.Status == 6) { }
    }

    private sealed class Fixture : IDisposable
    {
        private readonly string _root;
        public string DirectoryPath => _root;
        private readonly string _serverCertificate;
        private readonly string _serverKey;
        private readonly string _tokenAcl;
        private readonly string _principalAcl;
        private readonly string _clientCa;
        public string ServerRoot { get; }
        public string WrongServerRoot { get; }
        public Identity Writer { get; }
        public Identity Reader { get; }
        public Identity Stranger { get; }
        public Identity WrongEku { get; }
        public Identity WrongIssuer { get; }

        public Fixture()
        {
            _root = Path.Combine(Path.GetTempPath(), "rystem-dotnet-mtls-" +
                Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(_root);
            var existingServerFixture = Path.GetFullPath(Path.Combine(
                "src", "rystemdb-cli", "tests", "fixtures", "tls"));
            _serverCertificate = Path.Combine(_root, "server.pem");
            _serverKey = Path.Combine(_root, "server.key");
            File.Copy(Path.Combine(existingServerFixture, "server-cert.pem"), _serverCertificate);
            File.Copy(Path.Combine(existingServerFixture, "server-key.pem"), _serverKey);
            Private(_serverKey);
            ServerRoot = Path.Combine(existingServerFixture, "root-ca.pem");
            WrongServerRoot = Path.Combine(existingServerFixture, "wrong-root-ca.pem");
            _tokenAcl = Path.Combine(_root, "token.acl");
            _principalAcl = Path.Combine(_root, "principal.acl");
            File.WriteAllText(_tokenAcl,
                new string('1', 64) + " orders rw\n" +
                new string('2', 64) + " inventory rw\n");
            File.WriteAllText(_principalAcl,
                "rystem-mtls-principal-acl-v1\n" +
                "spiffe://rystemdb.local/client/writer orders rw\n" +
                "spiffe://rystemdb.local/client/reader orders r\n");
            Private(_tokenAcl);
            Private(_principalAcl);
            _clientCa = CreateCa("client").cert;
            var untrustedCa = CreateCa("untrusted");
            Writer = CreateIdentity("writer", "client",
                "spiffe://rystemdb.local/client/writer", "clientAuth");
            Reader = CreateIdentity("reader", "client",
                "spiffe://rystemdb.local/client/reader", "clientAuth");
            Stranger = CreateIdentity("stranger", "client",
                "spiffe://rystemdb.local/client/stranger", "clientAuth");
            WrongEku = CreateIdentity("wrong-eku", "client",
                "spiffe://rystemdb.local/client/writer", "serverAuth");
            WrongIssuer = CreateIdentity("wrong-issuer", "untrusted",
                "spiffe://rystemdb.local/client/writer", "clientAuth");
            _ = untrustedCa;
        }

        private (string cert, string key) CreateCa(string name)
        {
            var conf = Path.Combine(_root, name + "-ca.cnf");
            File.WriteAllText(conf,
                "[req]\ndistinguished_name=dn\nprompt=no\nx509_extensions=ca\n" +
                "[dn]\nCN=TEST ONLY " + name + "\n" +
                "[ca]\nbasicConstraints=critical,CA:TRUE\n" +
                "keyUsage=critical,keyCertSign,cRLSign\n");
            var key = Path.Combine(_root, name + "-ca.key");
            var cert = Path.Combine(_root, name + "-ca.pem");
            OpenSsl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "3",
                "-keyout", key, "-out", cert, "-config", conf);
            Private(key);
            return (cert, key);
        }

        private Identity CreateIdentity(string name, string ca, string principal, string eku)
        {
            var key = Path.Combine(_root, name + ".key");
            var csr = Path.Combine(_root, name + ".csr");
            var cert = Path.Combine(_root, name + ".pem");
            var conf = Path.Combine(_root, name + ".cnf");
            File.WriteAllText(conf,
                "[identity]\nbasicConstraints=critical,CA:FALSE\n" +
                "keyUsage=critical,digitalSignature\n" +
                "extendedKeyUsage=" + eku + "\n" +
                "subjectAltName=URI:" + principal + "\n");
            OpenSsl("req", "-new", "-newkey", "rsa:2048", "-nodes",
                "-keyout", key, "-out", csr, "-subj", "/CN=DO-NOT-TRUST-CN");
            OpenSsl("x509", "-req", "-in", csr, "-CA",
                Path.Combine(_root, ca + "-ca.pem"), "-CAkey",
                Path.Combine(_root, ca + "-ca.key"), "-CAcreateserial",
                "-out", cert, "-days", "2", "-extfile", conf, "-extensions", "identity");
            Private(key);
            return new Identity(cert, key);
        }

        private static void Private(string file)
        {
            if (!OperatingSystem.IsWindows())
                File.SetUnixFileMode(file, UnixFileMode.UserRead | UnixFileMode.UserWrite);
        }

        private static void OpenSsl(params string[] args)
        {
            var info = new ProcessStartInfo("openssl")
            {
                UseShellExecute = false, RedirectStandardOutput = true,
                RedirectStandardError = true, CreateNoWindow = true
            };
            foreach (string arg in args) info.ArgumentList.Add(arg);
            using var child = Process.Start(info)
                ?? throw new Exception("Could not start ephemeral OpenSSL fixture generator");
            if (!child.WaitForExit(20_000))
            {
                child.Kill(entireProcessTree: true);
                throw new Exception("Ephemeral OpenSSL generation timed out");
            }
            Require(child.ExitCode == 0, "Ephemeral OpenSSL fixture generation failed");
        }

        public async Task<NativeServer> StartAsync(string binary)
        {
            var info = new ProcessStartInfo(binary)
            {
                UseShellExecute = false, RedirectStandardOutput = true,
                RedirectStandardError = true
            };
            foreach (var arg in new[] { "rpc-serve", "--data", Path.Combine(_root, "store"),
                "--listen", "127.0.0.1:0", "--auth-file", _tokenAcl,
                "--tls-cert", _serverCertificate, "--tls-key", _serverKey,
                "--mtls-ca", _clientCa, "--mtls-policy", _principalAcl })
                info.ArgumentList.Add(arg);
            var child = Process.Start(info)
                ?? throw new Exception("Could not start actual Rust mTLS server");
            try
            {
                var banner = await child.StandardOutput.ReadLineAsync()
                    .WaitAsync(TimeSpan.FromSeconds(20));
                Require(banner is not null && banner.Contains("LOOPBACK RPC listening on "),
                    "Rust mTLS server did not start on loopback");
                var endpoint = IPEndPoint.Parse(banner!.Split(' ').Last());
                Require(IPAddress.IsLoopback(endpoint.Address),
                    "mTLS server unexpectedly escaped loopback");
                return new NativeServer(child, endpoint.Port);
            }
            catch
            {
                if (!child.HasExited) child.Kill(entireProcessTree: true);
                await child.WaitForExitAsync();
                child.Dispose();
                throw;
            }
        }

        public void Dispose()
        {
            if (Directory.Exists(_root)) Directory.Delete(_root, recursive: true);
        }
    }

    private sealed class NativeServer : IAsyncDisposable
    {
        private readonly Process _child;
        public int Port { get; }
        public NativeServer(Process child, int port) { _child = child; Port = port; }
        public async ValueTask DisposeAsync()
        {
            if (!_child.HasExited) _child.Kill(entireProcessTree: true);
            await _child.WaitForExitAsync();
            _child.Dispose();
        }
    }
}
