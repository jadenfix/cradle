using System;
using Beatbox;
using Xunit;

namespace Beatbox.Tests;

public class BaseUrlPolicyTests
{
    [Theory]
    [InlineData("https://api.example.test", "https://api.example.test")]
    [InlineData("https://api.example.test/root%3Aenv", "https://api.example.test/root%3Aenv")]
    [InlineData("https://api.example.test///", "https://api.example.test")]
    [InlineData("http://127.0.0.1:7300", "http://127.0.0.1:7300")]
    [InlineData("http://[::1]:7300", "http://[::1]:7300")]
    public void ValidateBaseUrl_accepts_safe_urls(string baseUrl, string expected)
    {
        Assert.Equal(expected, BeatboxClient.ValidateBaseUrl(baseUrl));
    }

    [Theory]
    [InlineData("")]
    [InlineData(" http://127.0.0.1:7300")]
    [InlineData("https://api.example.test ")]
    [InlineData("api.example.test")]
    [InlineData("ftp://api.example.test")]
    [InlineData("https://@api.example.test")]
    [InlineData("https://user:pass@api.example.test")]
    [InlineData("https://api.example.test/path?token=one")]
    [InlineData("https://api.example.test/path#frag")]
    [InlineData("http://localhost:7300")]
    [InlineData("http://api.example.test")]
    [InlineData("http://127.000.000.001:7300")]
    [InlineData("http://0177.0.0.1:7300")]
    [InlineData("http://2130706433:7300")]
    [InlineData("http://[0:0:0:0:0:0:0:1]:7300")]
    [InlineData("http://[::1]extra:7300")]
    [InlineData("http://[::1].evil.test:7300")]
    [InlineData("https://api.example.test/./v1")]
    [InlineData("https://api.example.test/../v1")]
    [InlineData("https://api.example.test/%2e/v1")]
    [InlineData("https://api.example.test/%2e%2e/v1")]
    [InlineData("https://api.example.test/api%2Fv1")]
    [InlineData("https://api.example.test/api%5Cv1")]
    [InlineData("https://api.example.test/api\\v1")]
    public void ValidateBaseUrl_rejects_unsafe_urls(string baseUrl)
    {
        Assert.Throws<ArgumentException>(() => BeatboxClient.ValidateBaseUrl(baseUrl));
    }

    [Fact]
    public void ValidateBaseUrl_rejects_null()
    {
        Assert.Throws<ArgumentException>(() => BeatboxClient.ValidateBaseUrl(null!));
    }

    [Fact]
    public void BuildRequestUri_preserves_base_prefix_and_encoded_job_segment()
    {
        var baseUrl = BeatboxClient.ValidateBaseUrl("https://api.example.test/root%3Aenv");
        var uri = BeatboxClient.BuildRequestUri(baseUrl, BeatboxClient.BuildJobPath("a/b"));

        Assert.Equal("https://api.example.test/root%3Aenv/v1/jobs/a%2Fb", uri.AbsoluteUri);
    }

    [Fact]
    public void BuildRequestUri_rejects_relative_paths()
    {
        var baseUrl = BeatboxClient.ValidateBaseUrl("https://api.example.test");

        Assert.Throws<ArgumentException>(() => BeatboxClient.BuildRequestUri(baseUrl, "v1/capabilities"));
    }

    [Fact]
    public void Http_handler_never_follows_redirects_or_uses_proxies()
    {
        using var handler = BeatboxClient.CreateHttpHandler();

        Assert.False(handler.AllowAutoRedirect);
        Assert.False(handler.UseProxy);
    }
}
