using System;
using Beatbox;
using Xunit;

namespace Beatbox.Tests;

public class JobUriTests
{
    [Theory]
    [InlineData("11111111-2222-3333-4444-555555555555", "/v1/jobs/11111111-2222-3333-4444-555555555555")]
    [InlineData("../execute", "/v1/jobs/..%2Fexecute")]
    [InlineData("x?k=v", "/v1/jobs/x%3Fk%3Dv")]
    [InlineData("a/b", "/v1/jobs/a%2Fb")]
    [InlineData("a b", "/v1/jobs/a%20b")]
    [InlineData("a#b", "/v1/jobs/a%23b")]
    public void BuildJobPath_encodes_id_as_single_segment(string id, string expected)
    {
        Assert.Equal(expected, BeatboxClient.BuildJobPath(id));
    }

    [Theory]
    [InlineData("")]
    [InlineData(".")]
    [InlineData("..")]
    public void BuildJobPath_rejects_retargeting_ids(string id)
    {
        Assert.Throws<ArgumentException>(() => BeatboxClient.BuildJobPath(id));
    }

    [Fact]
    public void BuildJobPath_rejects_null_id()
    {
        Assert.Throws<ArgumentNullException>(() => BeatboxClient.BuildJobPath(null!));
    }
}
