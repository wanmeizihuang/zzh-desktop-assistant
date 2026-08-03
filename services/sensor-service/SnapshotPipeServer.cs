using System.Buffers.Binary;
using System.IO.Pipes;
using System.Security.AccessControl;
using System.Security.Principal;
using System.Text.Json;

namespace ZZH.SensorService;

public sealed class SnapshotPipeServer
{
    private readonly string _pipeName;
    private readonly Func<SensorServiceSnapshot> _snapshot;
    private readonly TimeSpan _idleTimeout;

    public SnapshotPipeServer(
        string pipeName,
        Func<SensorServiceSnapshot> snapshot,
        TimeSpan? idleTimeout = null)
    {
        _pipeName = pipeName;
        _snapshot = snapshot;
        _idleTimeout = idleTimeout ?? TimeSpan.FromSeconds(30);
    }

    public async Task RunAsync(Action idle, CancellationToken cancellationToken)
    {
        DateTimeOffset lastClient = DateTimeOffset.UtcNow;
        while (!cancellationToken.IsCancellationRequested)
        {
            TimeSpan remainingIdle = _idleTimeout - (DateTimeOffset.UtcNow - lastClient);
            if (remainingIdle <= TimeSpan.Zero)
            {
                idle();
                return;
            }

            await using NamedPipeServerStream pipe = CreatePipe();
            using var waitCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            waitCancellation.CancelAfter(remainingIdle);
            try
            {
                await pipe.WaitForConnectionAsync(waitCancellation.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                idle();
                return;
            }

            lastClient = DateTimeOffset.UtcNow;
            try
            {
                await WriteSnapshotAsync(pipe, _snapshot(), cancellationToken).ConfigureAwait(false);
            }
            catch (IOException) when (!cancellationToken.IsCancellationRequested)
            {
                // A client may disappear between connecting and reading. Keep serving later clients.
            }
        }
    }

    private NamedPipeServerStream CreatePipe()
    {
        var security = new PipeSecurity();
        security.SetAccessRuleProtection(isProtected: true, preserveInheritance: false);
        security.AddAccessRule(new PipeAccessRule(
            new SecurityIdentifier(WellKnownSidType.LocalSystemSid, null),
            PipeAccessRights.FullControl,
            AccessControlType.Allow));
        security.AddAccessRule(new PipeAccessRule(
            new SecurityIdentifier(WellKnownSidType.BuiltinUsersSid, null),
            PipeAccessRights.Read,
            AccessControlType.Allow));

        return NamedPipeServerStreamAcl.Create(
            _pipeName,
            PipeDirection.Out,
            1,
            PipeTransmissionMode.Byte,
            PipeOptions.Asynchronous | PipeOptions.WriteThrough,
            0,
            SensorProtocol.MaximumSnapshotBytes,
            security);
    }

    private static async Task WriteSnapshotAsync(
        Stream stream,
        SensorServiceSnapshot snapshot,
        CancellationToken cancellationToken)
    {
        byte[] payload = JsonSerializer.SerializeToUtf8Bytes(snapshot, SensorProtocol.JsonOptions);
        if (payload.Length > SensorProtocol.MaximumSnapshotBytes)
        {
            payload = JsonSerializer.SerializeToUtf8Bytes(
                SensorServiceSnapshot.Failure("snapshot_too_large"),
                SensorProtocol.JsonOptions);
        }

        byte[] length = new byte[sizeof(int)];
        BinaryPrimitives.WriteInt32LittleEndian(length, payload.Length);
        await stream.WriteAsync(length, cancellationToken).ConfigureAwait(false);
        await stream.WriteAsync(payload, cancellationToken).ConfigureAwait(false);
        await stream.FlushAsync(cancellationToken).ConfigureAwait(false);
    }
}
