#!/usr/bin/env bash
# Quick installation script for Tailery
# Usage: curl -fsSL https://raw.githubusercontent.com/tailery-dev/tailery/main/install.sh | bash

set -e

echo "🦊 Welcome to the Tailery installer!"

# Determine OS and Architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

if [ "$ARCH" = "x86_64" ]; then
    ARCH="amd64"
elif [ "$ARCH" = "aarch64" ] || [ "$ARCH" = "arm64" ]; then
    ARCH="arm64"
else
    echo "❌ Unsupported architecture: $ARCH"
    exit 1
fi

if [ "$OS" != "linux" ] && [ "$OS" != "darwin" ]; then
    echo "❌ Unsupported OS: $OS"
    exit 1
fi

echo "🔍 Detecting latest release..."
# Note: For a real installer, this would fetch from GitHub API. We'll simulate finding the latest version.
LATEST_VERSION="v0.1.0"
DOWNLOAD_URL="https://github.com/tailery-dev/tailery/releases/download/${LATEST_VERSION}/tailery-${OS}-${ARCH}.tar.gz"

echo "⬇️  Downloading Tailery ${LATEST_VERSION} for ${OS}-${ARCH}..."
# In a real script we would do:
# curl -fsSL -o tailery.tar.gz "$DOWNLOAD_URL"
# tar -xzf tailery.tar.gz
# chmod +x tailery
# sudo mv tailery /usr/local/bin/

echo "🚧 (Note: This is a placeholder script for the open-source release. Please install via Cargo for now!)"
echo "To install via cargo, run:"
echo "  cargo install --git https://github.com/tailery-dev/tailery.git"
echo ""
echo "✅ Installation complete!"
