//! MCP tool-catalog drift guard.
//!
//! `tools/list` is a public client-facing surface. The committed fixture keeps
//! tool names, descriptions, and input schemas reviewable next to the OpenAPI
//! contract, while this test proves the fixture matches the server.
//!
//! Re-bless the fixture only when intentionally changing MCP tools:
//!
//! ```text
//! BEATBOX_BLESS_MCP_CATALOG=1 cargo test -p beatbox-server --test mcp_catalog_drift
//! ```

use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("mcp-tools.catalog.json")
}

#[test]
fn committed_mcp_catalog_matches_server() {
    let actual = beatbox_server::mcp_tool_catalog();
    let actual = match serde_json::to_string_pretty(&actual) {
        Ok(json) => json + "\n",
        Err(error) => panic!("MCP catalog should serialize: {error}"),
    };
    let path = fixture_path();

    if std::env::var("BEATBOX_BLESS_MCP_CATALOG").is_ok_and(|value| !value.is_empty()) {
        let Some(parent) = path.parent() else {
            panic!("fixture path should have parent: {}", path.display());
        };
        if let Err(error) = std::fs::create_dir_all(parent) {
            panic!("create MCP fixture directory {}: {error}", parent.display());
        }
        if let Err(error) = std::fs::write(&path, actual) {
            panic!(
                "write blessed MCP catalog fixture {}: {error}",
                path.display()
            );
        }
        eprintln!("blessed {}", path.display());
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read {} ({error}). Generate it with: \
             BEATBOX_BLESS_MCP_CATALOG=1 cargo test -p beatbox-server --test mcp_catalog_drift",
            path.display()
        )
    });
    assert_eq!(
        expected,
        actual,
        "\n\n{} is out of date with beatbox-server::mcp_tool_catalog().\n\
         Re-generate it with:\n  \
         BEATBOX_BLESS_MCP_CATALOG=1 cargo test -p beatbox-server --test mcp_catalog_drift\n",
        path.display()
    );
}
