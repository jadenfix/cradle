#!/usr/bin/env bash
# Fleet version-consistency gate.
#
# Every SDK manifest must declare the same version, and it must match the
# canonical version in sdks/openapi.json (info.version), which in turn is
# generated from the Rust workspace. This stops the fleet from drifting into a
# state where, say, the Python package is 0.2.0 while the Ruby gem is still
# 0.1.0.
#
# Usage: scripts/check-sdk-versions.sh
# Exits non-zero (and prints every mismatch) if versions disagree.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Canonical version = the spec the whole fleet is generated against.
canonical="$(python3 -c 'import json,sys; print(json.load(open("sdks/openapi.json"))["info"]["version"])')"

fail=0
check() {
  local lang="$1" file="$2" got="$3"
  if [[ -z "$got" ]]; then
    echo "MISSING  $lang: could not read a version from $file"
    fail=1
  elif [[ "$got" != "$canonical" ]]; then
    echo "MISMATCH $lang: $file declares $got, expected $canonical"
    fail=1
  else
    echo "ok       $lang: $got ($file)"
  fi
}

# TypeScript — package.json "version"
ts="$(python3 -c 'import json; print(json.load(open("sdks/typescript/package.json")).get("version",""))')"
check typescript sdks/typescript/package.json "$ts"

# Python — pyproject.toml [project] version
py="$(python3 -c 'import re; print((re.search(r"(?m)^version\s*=\s*\"([^\"]+)\"", open("sdks/python/pyproject.toml").read()) or [None,""]).__getitem__(1))')"
check python sdks/python/pyproject.toml "$py"

# Java — pom.xml project <version> (first one, the project's own)
java="$(python3 -c 'import re; s=open("sdks/java/pom.xml").read(); m=re.search(r"<artifactId>beatbox</artifactId>\s*<version>([^<]+)</version>", s); print(m.group(1) if m else "")')"
check java sdks/java/pom.xml "$java"

# Ruby — lib/beatbox/version.rb VERSION
rb="$(python3 -c 'import re; print((re.search(r"VERSION\s*=\s*\"([^\"]+)\"", open("sdks/ruby/lib/beatbox/version.rb").read()) or [None,""]).__getitem__(1))')"
check ruby sdks/ruby/lib/beatbox/version.rb "$rb"

# PHP — composer.json "version"
php="$(python3 -c 'import json; print(json.load(open("sdks/php/composer.json")).get("version",""))')"
check php sdks/php/composer.json "$php"

# C# — csproj <Version>
cs="$(python3 -c 'import re; print((re.search(r"<Version>([^<]+)</Version>", open("sdks/csharp/src/Beatbox/beatbox.csproj").read()) or [None,""]).__getitem__(1))')"
check csharp sdks/csharp/src/Beatbox/beatbox.csproj "$cs"

# Go has no manifest version (Go modules are versioned by git tag vN.N.N), so
# it is intentionally not checked here — the release workflow tags it.
echo "note     go: versioned by git tag (no manifest field)"

echo "----"
if [[ "$fail" -ne 0 ]]; then
  echo "FAIL: SDK versions are not consistent (canonical = $canonical)."
  exit 1
fi
echo "PASS: all SDK manifests at $canonical."
