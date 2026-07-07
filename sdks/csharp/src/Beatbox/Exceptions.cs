using System;

namespace Beatbox;

/// <summary>
/// Base class for every error raised by the beatbox SDK. Neither subclass ever
/// embeds auth material, so instances are safe to log or surface to users.
/// </summary>
public abstract class BeatboxException : Exception
{
    /// <summary>Creates a new <see cref="BeatboxException"/>.</summary>
    protected BeatboxException(string message, Exception? innerException = null)
        : base(message, innerException)
    {
    }
}

/// <summary>
/// Raised when the daemon returns a non-2xx HTTP response.
/// </summary>
public sealed class BeatboxApiException : BeatboxException
{
    /// <summary>HTTP status code of the response.</summary>
    public int Status { get; }

    /// <summary>
    /// Machine-readable code from the <c>{"error": {"code", "message"}}</c> body,
    /// or <see langword="null"/> if the body carried no code.
    /// </summary>
    public string? Code { get; }

    /// <summary>Creates a new <see cref="BeatboxApiException"/>.</summary>
    /// <param name="status">HTTP status code.</param>
    /// <param name="code">Error code from the response body, if any.</param>
    /// <param name="message">Human-readable message from the response body.</param>
    public BeatboxApiException(int status, string? code, string message)
        : base(Format(status, code, message))
    {
        Status = status;
        Code = code;
    }

    private static string Format(int status, string? code, string message)
        => string.IsNullOrEmpty(code)
            ? $"beatbox API error (HTTP {status}): {message}"
            : $"beatbox API error (HTTP {status}) [{code}]: {message}";
}

/// <summary>
/// Raised when the request never produced a usable HTTP response — connection
/// failures, DNS errors, timeouts, and malformed response bodies.
/// </summary>
public sealed class BeatboxTransportException : BeatboxException
{
    /// <summary>Creates a new <see cref="BeatboxTransportException"/>.</summary>
    public BeatboxTransportException(string message, Exception? innerException = null)
        : base($"beatbox transport error: {message}", innerException)
    {
    }
}
