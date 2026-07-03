# Releasing the SDK fleet

The fleet ships as one coordinated release: every SDK carries the same version,
which matches the API version in [`openapi.json`](./openapi.json). Releases are
manual, dry-run by default, and gated behind a human approval.

## Prerequisites (one-time repo setup)

1. **Create a `release` GitHub Environment** (Settings → Environments) and add
   **required reviewers**. The `publish` job in
   [`sdk-release.yml`](../.github/workflows/sdk-release.yml) targets this
   environment, so no push to any registry happens until a reviewer approves the
   run. This is the rollout gate.

2. **Add registry secrets** to that environment. Each is optional — if a secret
   is absent, that language's publish step logs a warning and is skipped, never
   failing the run — so you can roll out one language at a time.

   | Language | Secret(s) | Registry |
   | --- | --- | --- |
   | TypeScript | `NPM_TOKEN` | npmjs.org |
   | Python | `PYPI_API_TOKEN` | pypi.org |
   | Java | `OSSRH_USERNAME`, `OSSRH_TOKEN`, `MAVEN_GPG_PRIVATE_KEY` | Maven Central |
   | Ruby | `RUBYGEMS_API_KEY` | rubygems.org |
   | C# | `NUGET_API_KEY` | nuget.org |
   | Go | — | published by git tag `sdks/go/vX.Y.Z` |
   | PHP | — | Packagist syncs from git tag `sdks/php/vX.Y.Z` |

   > Maven Central additionally needs `<distributionManagement>` and a GPG
   > signing profile in `sdks/java/pom.xml`; those are left out of the committed
   > pom because they are account-specific.

## Cutting a release

1. **Bump the version** in every manifest so they agree with `openapi.json`.
   The gate that enforces this is `scripts/check-sdk-versions.sh`; the files it
   reads are the canonical places to edit:

   - `sdks/typescript/package.json` → `version`
   - `sdks/python/pyproject.toml` → `version`
   - `sdks/java/pom.xml` → project `<version>`
   - `sdks/ruby/lib/beatbox/version.rb` → `VERSION`
   - `sdks/php/composer.json` → `version`
   - `sdks/csharp/src/Beatbox/beatbox.csproj` → `<Version>`
   - Go has no manifest version — it is released by the tag the workflow pushes.

   The API version in `openapi.json` is the `info(version = …)` literal on
   `ApiDoc` in `crates/beatbox-server/src/lib.rs` (it is intentionally
   independent of the Rust crate version). Bump that literal too if the release
   changes the API surface, and re-bless the spec:

   ```bash
   BEATBOX_BLESS_OPENAPI=1 cargo test -p beatbox-server --test openapi_drift
   ```

2. **Land the bump** through a normal PR. `sdk-ci` will fail the
   version-consistency job if anything disagrees.

3. **Dry run.** Actions → `sdk-release` → *Run workflow*:
   - `version`: the new version (must match the manifests).
   - `languages`: `all` or a single language.
   - `dry_run`: **true**.

   This validates, builds every artifact, and uploads them to the run summary.
   Nothing is published. Inspect the artifacts.

4. **Real release.** Re-run with `dry_run: false`. The `publish` job pauses for
   environment approval; once approved, each language publishes to its registry
   (or is skipped if its secret is unset). Go and PHP are released by pushing a
   `sdks/<lang>/vX.Y.Z` tag.

## Why a drift guard, not codegen

The SDKs are hand-written for idiomatic ergonomics, but they must never describe
an API the daemon doesn't implement. `openapi.json` is generated from the server
and checked in; the `openapi_drift` test fails CI if the checked-in spec and the
server disagree. So the spec is always true, and the SDKs are reviewed against a
spec that is always true. See [`README.md`](./README.md) for the full pipeline.
