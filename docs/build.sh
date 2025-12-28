#!/usr/bin/env bash
set -euo pipefail

REPO="resolveworks/insight"
GENERATED_DIR="docs/src/generated"
OUTPUT_FILE="$GENERATED_DIR/downloads.html"

mkdir -p "$GENERATED_DIR"

RELEASE_JSON=$(gh api "repos/$REPO/releases/latest" 2>/dev/null || echo "")

# Check if we got a valid release (not empty, not a 404 error)
if [ -z "$RELEASE_JSON" ] || echo "$RELEASE_JSON" | jq -e '.message == "Not Found"' > /dev/null 2>&1; then
    # No releases - generate placeholder
    cat > "$OUTPUT_FILE" << 'EOF'
<div class="download-section">
  <p>No releases available yet. Check <a href="https://github.com/resolveworks/insight/releases">GitHub releases</a>.</p>
</div>
EOF
    echo "No releases found, generated placeholder"
    exit 0
fi

VERSION=$(echo "$RELEASE_JSON" | jq -r '.tag_name')

# Generate HTML for each platform asset
ASSETS_HTML=$(echo "$RELEASE_JSON" | jq -r '
  .assets // [] |
  map(select(.name | test("\\.(dmg|msi|exe|AppImage|deb)$"; "i"))) |
  map({
    url: .browser_download_url,
    platform: (
      if (.name | test("aarch64.*\\.dmg$"; "i")) then "macos"
      elif (.name | test("x64.*\\.dmg$|x86_64.*\\.dmg$"; "i")) then "macos"
      elif (.name | test("\\.dmg$"; "i")) then "macos"
      elif (.name | test("\\.msi$|\\.exe$"; "i")) then "windows"
      elif (.name | test("\\.AppImage$|\\.deb$"; "i")) then "linux"
      else "other"
      end
    ),
    label: (
      if (.name | test("aarch64.*\\.dmg$"; "i")) then "macOS (Apple Silicon)"
      elif (.name | test("x64.*\\.dmg$|x86_64.*\\.dmg$"; "i")) then "macOS (Intel)"
      elif (.name | test("\\.dmg$"; "i")) then "macOS"
      elif (.name | test("\\.msi$"; "i")) then "Windows"
      elif (.name | test("\\.exe$"; "i")) then "Windows"
      elif (.name | test("\\.AppImage$"; "i")) then "Linux (AppImage)"
      elif (.name | test("\\.deb$"; "i")) then "Linux (Debian)"
      else "Download"
      end
    )
  }) |
  map("<a href=\"\(.url)\" class=\"download-btn\" data-platform=\"\(.platform)\">\(.label)</a>") |
  join("\n    ")
')

if [ -z "$ASSETS_HTML" ]; then
    # Release exists but no matching assets
    cat > "$OUTPUT_FILE" << EOF
<div class="download-section">
  <p class="release-version">Latest version: <strong>$VERSION</strong></p>
  <p>Downloads coming soon. Check the <a href="https://github.com/$REPO/releases/tag/$VERSION">release page</a>.</p>
</div>
EOF
    echo "Generated placeholder for $VERSION (no assets)"
else
    cat > "$OUTPUT_FILE" << EOF
<div class="download-section">
  <p class="release-version">Latest version: <strong>$VERSION</strong></p>
  <div class="download-buttons">
    $ASSETS_HTML
  </div>
</div>
EOF
    echo "Generated download links for $VERSION"
fi
