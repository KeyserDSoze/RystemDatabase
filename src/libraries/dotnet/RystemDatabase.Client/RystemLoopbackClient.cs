using System.Net;

namespace RystemDatabase.Client;

/// <summary>RED test scaffold: experimental localhost-only TCP client.</summary>
public sealed class RystemLoopbackClient : IAsyncDisposable
{
    public static Task<RystemLoopbackClient> ConnectAsync(
        IPAddress address, int port, CancellationToken cancellationToken = default)
        => throw new NotSupportedException("TCP client not implemented");

    public Task PingAsync(CancellationToken cancellationToken = default)
        => throw new NotSupportedException("PING not implemented");

    public Task PutAsync(string collection, string id, RdbValue.Object document,
        CancellationToken cancellationToken = default)
        => throw new NotSupportedException("PUT not implemented");

    public Task<RdbValue.Object?> GetAsync(string collection, string id,
        CancellationToken cancellationToken = default)
        => throw new NotSupportedException("GET not implemented");

    public Task<bool> DeleteAsync(string collection, string id,
        CancellationToken cancellationToken = default)
        => throw new NotSupportedException("DELETE not implemented");

    public ValueTask DisposeAsync() => ValueTask.CompletedTask;
}
