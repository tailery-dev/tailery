#!/usr/bin/env bash
# ==============================================================================
# Tailery Installation Script
# Usage: curl -fsSL https://raw.githubusercontent.com/tailery-dev/tailery/main/install.sh | bash
# ==============================================================================

set -e

REPO="tailery-dev/tailery"
BINARY_NAME="tailery"

echo "🦊 Welcome to the Tailery installer!"

# 1. Detect OS
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
case "$OS" in
    darwin)
        OS_NAME="macos"
        ;;
    linux)
        OS_NAME="linux"
        ;;
    *)
        echo "❌ Unsupported operating system: $OS"
        echo "Tailery currently supports macOS and Linux. For Windows, please build from source or use cargo."
        exit 1
        ;;
esac

# 2. Detect Architecture
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64)
        ARCH_NAME="x86_64"
        ;;
    aarch64|arm64)
        ARCH_NAME="arm64"
        ;;
    *)
        echo "❌ Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

echo "🖥️  Detected platform: ${OS_NAME} (${ARCH_NAME})"

# 3. Detect SHA256 Tool
if command -v sha256sum >/dev/null 2>&1; then
    SHA_CMD="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
    SHA_CMD="shasum -a 256"
else
    echo "⚠ Warning: Neither 'sha256sum' nor 'shasum' was found. Checksum verification will be skipped."
    SHA_CMD=""
fi

# 4. Determine Version to Install
if [ -n "$TAILERY_VERSION" ]; then
    VERSION="$TAILERY_VERSION"
    echo "📌 Using requested version: $VERSION"
else
    echo "🔍 Resolving latest release from GitHub..."
    LATEST_TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)
    if [ -z "$LATEST_TAG" ]; then
        echo "⚠ Could not determine latest version from GitHub API, falling back to v0.1.0"
        VERSION="v0.1.0"
    else
        VERSION="$LATEST_TAG"
    fi
    echo "📦 Latest version: $VERSION"
fi

VERSION_CLEAN="${VERSION#v}"

# 5. Determine Download URL candidates (supporting both target-triple and os-arch asset conventions)
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

DOWNLOAD_URLS=(
    "https://github.com/${REPO}/releases/download/${VERSION}/${BINARY_NAME}-${VERSION_CLEAN}-${OS_NAME}-${ARCH_NAME}.tar.gz"
    "https://github.com/${REPO}/releases/download/${VERSION}/${BINARY_NAME}-${VERSION}-${OS_NAME}-${ARCH_NAME}.tar.gz"
    "https://github.com/${REPO}/releases/download/${VERSION}/${BINARY_NAME}-v${VERSION_CLEAN}-${OS_NAME}-${ARCH_NAME}.tar.gz"
)

TARBALL_PATH="$TMP_DIR/tailery.tar.gz"
CHECKSUM_PATH="$TMP_DIR/tailery.sha256"
DOWNLOADED=false

for url in "${DOWNLOAD_URLS[@]}"; do
    if curl -fsSL -o "$TARBALL_PATH" "$url" 2>/dev/null; then
        DOWNLOAD_URL="$url"
        DOWNLOADED=true
        break
    fi
done

if [ "$DOWNLOADED" = false ]; then
    echo "❌ Failed to download Tailery archive from GitHub releases."
    echo "Tried URLs:"
    for url in "${DOWNLOAD_URLS[@]}"; do
        echo "  - $url"
    done
    echo ""
    echo "You can build directly from source using:"
    echo "  cargo install --git https://github.com/${REPO}.git"
    exit 1
fi

echo "⬇️  Downloaded package from $DOWNLOAD_URL"

# 6. Verify Checksum if available
if [ -n "$SHA_CMD" ]; then
    CHECKSUM_URL="${DOWNLOAD_URL}.sha256"
    if curl -fsSL -o "$CHECKSUM_PATH" "$CHECKSUM_URL" 2>/dev/null; then
        echo "🔒 Verifying SHA256 cryptographic checksum..."
        EXPECTED_HASH=$(awk '{print $1}' "$CHECKSUM_PATH" | tr -d ' \r\n')
        ACTUAL_HASH=$($SHA_CMD "$TARBALL_PATH" | awk '{print $1}' | tr -d ' \r\n')
        if [ "$EXPECTED_HASH" != "$ACTUAL_HASH" ]; then
            echo "❌ Checksum verification failed!"
            echo "  Expected: $EXPECTED_HASH"
            echo "  Actual:   $ACTUAL_HASH"
            exit 1
        fi
        echo "✔ Checksum verified successfully ($ACTUAL_HASH)"
    else
        echo "ℹ️  No remote checksum file found at ${CHECKSUM_URL}, skipping checksum check."
    fi
fi

# 7. Extract archive
echo "📂 Extracting binary..."
tar -xzf "$TARBALL_PATH" -C "$TMP_DIR"

if [ -f "$TMP_DIR/$BINARY_NAME" ]; then
    EXTRACTED_BIN="$TMP_DIR/$BINARY_NAME"
else
    EXTRACTED_BIN=$(find "$TMP_DIR" -type f -name "$BINARY_NAME" -perm -111 | head -n 1)
fi

if [ -z "$EXTRACTED_BIN" ] || [ ! -f "$EXTRACTED_BIN" ]; then
    # Fallback to any executable named tailery
    EXTRACTED_BIN=$(find "$TMP_DIR" -type f -name "$BINARY_NAME" | head -n 1)
fi

if [ -z "$EXTRACTED_BIN" ] || [ ! -f "$EXTRACTED_BIN" ]; then
    echo "❌ Could not find '$BINARY_NAME' executable inside downloaded archive."
    exit 1
fi

chmod +x "$EXTRACTED_BIN"

# 8. Select Install Destination
INSTALL_DIR=""
if [ -w "/usr/local/bin" ]; then
    INSTALL_DIR="/usr/local/bin"
elif [ -d "$HOME/.local/bin" ] || mkdir -p "$HOME/.local/bin" 2>/dev/null; then
    INSTALL_DIR="$HOME/.local/bin"
else
    INSTALL_DIR="/usr/local/bin"
fi

TARGET_BIN="$INSTALL_DIR/$BINARY_NAME"
echo "🚀 Installing $BINARY_NAME to $TARGET_BIN..."

if [ -w "$INSTALL_DIR" ]; then
    mv "$EXTRACTED_BIN" "$TARGET_BIN"
else
    echo "🔒 Permission required to write to $INSTALL_DIR (using sudo)..."
    sudo mv "$EXTRACTED_BIN" "$TARGET_BIN"
fi

chmod +x "$TARGET_BIN"

# 9. Verify Installation & Path
echo ""
echo "✨ Installation successful!"
if "$TARGET_BIN" --version >/dev/null 2>&1; then
    "$TARGET_BIN" --version
fi

case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        ;;
    *)
        echo ""
        echo "⚠ Note: $INSTALL_DIR is not in your current PATH."
        echo "Add the following line to your shell profile (~/.zshrc, ~/.bashrc, or ~/.config/fish/config.fish):"
        echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
esac

echo ""
echo "🎉 Run 'tailery' to start the interactive TUI, or 'tailery doctor' to run diagnostics!"
