using System.Buffers.Binary;
using System.IO;
using System.Net;
using System.Net.Sockets;

namespace RystemDatabase.Client;

/// <summary>
/// Protocol-level error returned by the experimental localhost Rust server.
/// A status code is never converted into a filesystem/secret-bearing message.
/// </summary>
public sealed class RystemRpcException : Exception
{
    public ushort Status { get; }

    public RystemRpcException(ushort status)
        : base($"Rystem Database RPC rejected the operation (status {status})")
        => Status = status;
}

/// <summary>
/// A single, persistent, asynchronous localhost-only RSWP v1 TCP connection.
/// Optional local-token HELLO is supported, but NO TLS: non-loopback rejected.
/// All requests share an I/O gate; this milestone is not yet a pooled or
/// multiplexed production SDK. Failed/uncertain writes are never auto-retried.
/// </summary>
public sealed class RystemLoopbackClient : IAsyncDisposable
{
    private const int MaxPayload = RystemWireCodec.MaxPayloadBytes;
    private static readonly TimeSpan DefaultTimeout = TimeSpan.FromSeconds(3);
    private readonly TcpClient _tcp;
    private readonly NetworkStream _stream;
    private readonly SemaphoreSlim _gate = new(1, 1);
    private ulong _nextId = 1;
    private bool _disposed;

    private RystemLoopbackClient(TcpClient tcp)
    {
        _tcp = tcp;
        _stream = tcp.GetStream();
    }

    public static Task<RystemLoopbackClient> ConnectAsync(
        IPAddress address, int port, CancellationToken cancellationToken = default)
        => ConnectInternalAsync(address, port, null, cancellationToken);

    /// <summary>
    /// Authenticated HELLO over localhost only; accepts a raw 32-byte secret.
    /// It is a developer-mode credential, NOT encrypted TLS authentication.
    /// Do not log the token or pass it via command line arguments.
    /// </summary>
    public static Task<RystemLoopbackClient> ConnectWithTokenAsync(
        IPAddress address, int port, byte[] token, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(token);
        if (token.Length != 32)
            throw new ArgumentException("RSWP local token must be 32 bytes", nameof(token));
        // Defensively copy before any asynchronous socket operation.
        return ConnectInternalAsync(address, port, token.ToArray(), cancellationToken);
    }

    private static async Task<RystemLoopbackClient> ConnectInternalAsync(
        IPAddress address, int port, byte[]? token, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(address);
        if (!IPAddress.IsLoopback(address))
            throw new ArgumentException("Rystem RPC requires a loopback address until TLS/auth exists", nameof(address));
        if (port is < 1 or > 65535)
            throw new ArgumentOutOfRangeException(nameof(port));
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(DefaultTimeout);
        var tcp = new TcpClient(address.AddressFamily) { NoDelay = true };
        try
        {
            await tcp.ConnectAsync(address, port, timeout.Token).ConfigureAwait(false);
            var client = new RystemLoopbackClient(tcp);
            byte[] hello = [1, 0];
            if (token is not null)
            {
                hello = new byte[36];
                hello[0] = 1;
                hello[1] = 0;
                hello[2] = 0;
                hello[3] = 32;
                token.CopyTo(hello, 4);
            }
            var reply = await client.CallAsync(WireOperation.Hello, hello, cancellationToken)
                .ConfigureAwait(false);
            if (reply.Status != 0)
                throw new RystemRpcException(reply.Status);
            if (!reply.Payload.AsSpan().SequenceEqual(new byte[] { 1, 0 }))
                throw new InvalidDataException("Rust server returned invalid RSWP 1.0 HELLO response");
            return client;
        }
        catch
        {
            tcp.Dispose();
            throw;
        }
    }

    public async Task PingAsync(CancellationToken cancellationToken = default)
    {
        var reply = await CallAsync(WireOperation.Ping, [], cancellationToken).ConfigureAwait(false);
        EnsureStatus(reply, 0);
        if (reply.Payload.Length != 0)
            throw new InvalidDataException("Nonempty RSWP PING response");
    }

    public async Task PutAsync(string collection, string id, RdbValue.Object document,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(document);
        byte[] key = RystemCrudKey.Encode(collection, id);
        byte[] body = RystemRdbCodec.Encode(document);
        if (key.Length + body.Length > MaxPayload)
            throw new ArgumentException("Rystem RPC PUT payload too large");
        byte[] payload = new byte[key.Length + body.Length];
        key.CopyTo(payload, 0);
        body.CopyTo(payload, key.Length);
        var reply = await CallAsync(WireOperation.Put, payload, cancellationToken).ConfigureAwait(false);
        EnsureStatus(reply, 0);
        if (reply.Payload.Length != 0)
            throw new InvalidDataException("Nonempty RSWP PUT response");
    }

    public async Task<RdbValue.Object?> GetAsync(string collection, string id,
        CancellationToken cancellationToken = default)
    {
        var reply = await CallAsync(WireOperation.Get,
            RystemCrudKey.Encode(collection, id), cancellationToken).ConfigureAwait(false);
        if (reply.Status == 1 && reply.Payload.Length == 0) return null;
        EnsureStatus(reply, 0);
        if (RystemRdbCodec.Decode(reply.Payload) is not RdbValue.Object result)
            throw new InvalidDataException("Expected a typed binary RDB object response");
        return result;
    }

    public async Task<bool> DeleteAsync(string collection, string id,
        CancellationToken cancellationToken = default)
    {
        var reply = await CallAsync(WireOperation.Delete,
            RystemCrudKey.Encode(collection, id), cancellationToken).ConfigureAwait(false);
        if (reply.Payload.Length != 0)
            throw new InvalidDataException("Nonempty RSWP DELETE response");
        if (reply.Status == 1) return false;
        EnsureStatus(reply, 0);
        return true;
    }

    private static void EnsureStatus(WireFrame response, ushort expected)
    {
        if (response.Status != expected)
            throw new RystemRpcException(response.Status);
    }

    private async Task<WireFrame> CallAsync(WireOperation operation, byte[] payload,
        CancellationToken cancellationToken)
    {
        if (_disposed) throw new ObjectDisposedException(nameof(RystemLoopbackClient));
        if (payload.Length > MaxPayload) throw new ArgumentException("RSWP payload too large");
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        deadline.CancelAfter(DefaultTimeout);

        await _gate.WaitAsync(deadline.Token).ConfigureAwait(false);
        try
        {
            if (_disposed) throw new ObjectDisposedException(nameof(RystemLoopbackClient));
            ulong requestId = checked(_nextId++);
            var bytes = RystemWireCodec.Encode(new WireFrame(
                WireKind.Request, operation, requestId, 0, payload));
            await _stream.WriteAsync(bytes, deadline.Token).ConfigureAwait(false);

            var header = new byte[RystemWireCodec.HeaderLength];
            await _stream.ReadExactlyAsync(header, deadline.Token).ConfigureAwait(false);
            uint count = BinaryPrimitives.ReadUInt32BigEndian(header.AsSpan(20, 4));
            if (count > MaxPayload)
                throw new InvalidDataException("Incoming RSWP body too large");

            var raw = new byte[checked(header.Length + (int)count)];
            header.CopyTo(raw, 0);
            if (count > 0)
                await _stream.ReadExactlyAsync(raw.AsMemory(header.Length, (int)count), deadline.Token)
                    .ConfigureAwait(false);
            // Reuse the tested canonical codec for all other header validation.
            var reply = RystemWireCodec.Decode(raw);
            if (reply.Kind != WireKind.Response ||
                reply.Operation != operation || reply.RequestId != requestId)
                throw new InvalidDataException("RSWP response correlation mismatch");
            return reply;
        }
        catch (Exception ex) when (ex is IOException or OperationCanceledException
            or SocketException or InvalidDataException or ArgumentException)
        {
            // After a partial TCP write or read, the outcome may be unknown.
            // Close the connection; never silently replay a write or reuse
            // a potentially desynchronized frame stream.
            _tcp.Dispose();
            throw;
        }
        finally
        {
            _gate.Release();
        }
    }

    public async ValueTask DisposeAsync()
    {
        if (_disposed) return;
        await _gate.WaitAsync().ConfigureAwait(false);
        try
        {
            if (_disposed) return;
            _disposed = true;
            await _stream.DisposeAsync().ConfigureAwait(false);
            _tcp.Dispose();
        }
        finally
        {
            _gate.Release();
        }
        // The semaphore is intentionally not disposed: callers completing a
        // concurrently queued request should observe ObjectDisposedException
        // rather than a disposal race from this synchronization primitive.
    }
}
