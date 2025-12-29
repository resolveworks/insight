#!/usr/bin/env bash
set -euo pipefail

REPO="resolveworks/insight"
GENERATED_DIR="docs/src/generated"
OUTPUT_FILE="$GENERATED_DIR/downloads.html"

mkdir -p "$GENERATED_DIR"

RELEASE_JSON=$(gh api "repos/$REPO/releases/latest" 2>/dev/null || echo "")

# Check if we got a valid release (not empty, not a 404 error)
if [ -z "$RELEASE_JSON" ] || echo "$RELEASE_JSON" | jq -e '.message == "Not Found"' > /dev/null 2>&1; then
    cat > "$OUTPUT_FILE" << 'EOF'
<div class="download-section">
  <p>No releases available yet. Check <a href="https://github.com/resolveworks/insight/releases">GitHub releases</a>.</p>
</div>
EOF
    echo "No releases found, generated placeholder"
    exit 0
fi

VERSION=$(echo "$RELEASE_JSON" | jq -r '.tag_name')

# Generate table rows for all assets
ROWS=$(echo "$RELEASE_JSON" | jq -r '
  .assets // [] |
  map("<tr><td><a href=\"\(.browser_download_url)\">\(.name)</a></td></tr>") |
  join("\n")
')

if [ -z "$ROWS" ]; then
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
  <table class="download-table">
    <tbody>
$ROWS
    </tbody>
  </table>
</div>
EOF
    echo "Generated download links for $VERSION"
fi
