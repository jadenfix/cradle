//! Spec-drift guard.
//!
//! The SDK fleet in `sdks/` is generated/synced against `sdks/openapi.json`.
//! That file must stay identical to the spec the server actually serves,
//! otherwise the SDKs describe an API the daemon does not implement.
//!
//! This test regenerates the canonical document from the live `ApiDoc` and
//! asserts it matches the committed `sdks/openapi.json` byte-for-byte.
//!
//! When you intentionally change the API surface, re-bless the checked-in
//! spec (which the SDK CI then drift-checks the generated models against):
//!
//! ```text
//! BEATBOX_BLESS_OPENAPI=1 cargo test -p beatbox-server --test openapi_drift
//! ```

use std::path::PathBuf;

/// `sdks/openapi.json`, resolved from this crate's manifest dir so the test is
/// independent of the process working directory.
fn spec_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("sdks")
        .join("openapi.json")
}

#[test]
fn committed_openapi_matches_server() {
    let generated = beatbox_server::openapi_spec_json();
    let path = spec_path();

    // Re-bless only on a non-empty value, so `BEATBOX_BLESS_OPENAPI=` (empty)
    // does not silently overwrite the committed spec.
    if std::env::var("BEATBOX_BLESS_OPENAPI").is_ok_and(|v| !v.is_empty()) {
        if let Err(e) = std::fs::write(&path, &generated) {
            panic!("failed to write blessed {}: {e}", path.display());
        }
        eprintln!("blessed {}", path.display());
        return;
    }

    let committed = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => panic!(
            "cannot read {} ({e}). Generate it with: \
             BEATBOX_BLESS_OPENAPI=1 cargo test -p beatbox-server --test openapi_drift",
            path.display()
        ),
    };

    assert_eq!(
        committed,
        generated,
        "\n\n{} is out of date with the server's OpenAPI document.\n\
         Re-generate it and re-sync the SDKs:\n  \
         BEATBOX_BLESS_OPENAPI=1 cargo test -p beatbox-server --test openapi_drift\n",
        spec_path().display()
    );
}
