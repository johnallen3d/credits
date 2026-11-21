#!/usr/bin/env bash
# Extracts the App Intents metadata (Siri, Shortcuts, Spotlight) from the last
# `swift build -c release` and checks it against Metadata.appintents, which is
# committed because the Nix build has no Xcode to extract it. With
# UPDATE_INTENTS=1 it rewrites the committed copy instead.
#
# Needs Xcode (for appintentsmetadataprocessor). Run from macos/.
set -euo pipefail

build=.build/out/Intermediates.noindex/CreditsLauncher.build/Release/CreditsLauncher-p.build/Objects-normal/arm64
[[ -f "$build/CreditsLauncher.SwiftFileList" ]] || { echo "No release build in $build; run swift build -c release" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
find "$PWD/$build" -name '*.swiftconstvalues' > "$tmp/constvalues"

swift=$(xcrun --find swift)
xcrun appintentsmetadataprocessor --force --quiet-warnings \
  --output "$tmp" \
  --toolchain-dir "${swift%/usr/bin/swift}" \
  --module-name CreditsLauncher \
  --bundle-identifier "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' Info.plist)" \
  --sdk-root "$(xcrun --show-sdk-path)" \
  --xcode-version "$(xcodebuild -version | awk '/Build version/ {print $3}')" \
  --platform-family macOS \
  --deployment-target "$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' Info.plist)" \
  --target-triple arm64-apple-macosx \
  --source-file-list "$build/CreditsLauncher.SwiftFileList" \
  --swift-const-vals-list "$tmp/constvalues" >"$tmp/log" 2>&1 || { cat "$tmp/log" >&2; exit 1; }

# The processor writes some lists in no fixed order from run to run, so
# compare with every list sorted. (That misses a pure reordering, such as
# swapped phrases, which doesn't matter here.)
normalize() {
  /usr/bin/python3 -I -c '
import json, sys
def norm(v):
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    if isinstance(v, list):
        return sorted((norm(x) for x in v), key=lambda x: json.dumps(x, sort_keys=True))
    return v
for path in sys.argv[1:]:
    print(json.dumps(norm(json.load(open(path))), sort_keys=True))
' "$1/extract.actionsdata" "$1/version.json"
}

if [[ "${UPDATE_INTENTS:-}" == 1 ]]; then
  rm -rf Metadata.appintents
  cp -R "$tmp/Metadata.appintents" .
  echo "Updated macos/Metadata.appintents"
elif ! diff <(normalize Metadata.appintents) <(normalize "$tmp/Metadata.appintents") >/dev/null; then
  echo "macos/Metadata.appintents is out of date with the intents (or Xcode); run: mise run mac:intents" >&2
  exit 1
fi
