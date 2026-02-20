#!/usr/bin/env bash
set -euo pipefail

GITHUB_REPO="${GOLDBOT_GITHUB_REPO:-GOLDhjy/GoldBot}"
REPO_URL="${GOLDBOT_REPO_URL:-https://github.com/GOLDhjy/GoldBot.git}"
VERSION="${GOLDBOT_VERSION:-latest}"
MODE="binary"
BIN_DIR="${INSTALL_BIN_DIR:-$HOME/.local/bin}"
SCRIPT_SOURCE="${BASH_SOURCE[0]-$0}"
TMP_DIRS=()

cleanup_tmp_dirs() {
  local dir
  for dir in "${TMP_DIRS[@]-}"; do
    [[ -n "$dir" && -d "$dir" ]] && rm -rf "$dir"
  done || true
  return 0
}

register_tmp_dir() {
  TMP_DIRS+=("$1")
}

trap cleanup_tmp_dirs EXIT

usage() {
  cat <<'USAGE'
Usage: bash scripts/install.sh [options]

Options:
  --version <tag>   Install a specific release tag (e.g. v0.2.0). Default: latest
  --source          Build from source instead of downloading prebuilt binary
  --repo <git-url>  Source repository URL used with --source
USAGE
}

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing required command: $cmd"
    exit 1
  fi
}

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin)
      case "$arch" in
        arm64|aarch64) echo "macos-aarch64" ;;
        x86_64) echo "macos-x86_64" ;;
        *) return 1 ;;
      esac
      ;;
    Linux)
      case "$arch" in
        x86_64) echo "linux-x86_64" ;;
        *) return 1 ;;
      esac
      ;;
    *)
      return 1
      ;;
  esac
}

latest_tag() {
  local resolved
  resolved="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/$GITHUB_REPO/releases/latest")"
  printf '%s\n' "${resolved##*/}"
}

download_binary() {
  local version="$1"
  local target="$2"
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  register_tmp_dir "$tmp_dir"

  local filename="goldbot-${version#v}-${target}.tar.gz"
  local download_url="https://github.com/$GITHUB_REPO/releases/download/${version}/${filename}"

  echo "Downloading $download_url"
  require_cmd curl
  curl -fsSL "$download_url" -o "$tmp_dir/$filename"
  
  tar -xzf "$tmp_dir/$filename" -C "$tmp_dir"
  
  mkdir -p "$BIN_DIR"
  mv "$tmp_dir/goldbot" "$BIN_DIR/goldbot"
  chmod +x "$BIN_DIR/goldbot"
  echo "Installed to $BIN_DIR/goldbot"
}

build_from_source() {
  local version="$1"
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  register_tmp_dir "$tmp_dir"

  echo "Cloning $REPO_URL"
  require_cmd git
  git clone --depth 1 --branch "$version" "$REPO_URL" "$tmp_dir/repo"
  
  cd "$tmp_dir/repo"
  require_cmd cargo
  cargo install --path . --root "$tmp_dir/install"
  
  mkdir -p "$BIN_DIR"
  mv "$tmp_dir/install/bin/goldbot" "$BIN_DIR/goldbot"
  echo "Installed to $BIN_DIR/goldbot"
}

main() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --version)
        VERSION="$2"
        shift 2
        ;;
      --source)
        MODE="source"
        shift
        ;;
      --repo)
        REPO_URL="$2"
        shift 2
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        echo "Unknown option: $1"
        usage
        exit 1
        ;;
    esac
  done

  if [[ "$VERSION" == "latest" ]]; then
    VERSION="$(latest_tag)"
    echo "Latest version: $VERSION"
  fi

  if [[ "$MODE" == "binary" ]]; then
    target="$(detect_target)" || {
      echo "Failed to detect target platform. Use --source to build from source."
      exit 1
    }
    echo "Target: $target"
    download_binary "$VERSION" "$target"
  else
    build_from_source "$VERSION"
  fi

  if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo ""
    echo "Add $BIN_DIR to your PATH:"
    echo "  echo 'export PATH=\"\$PATH:$BIN_DIR\"' >> ~/.bashrc  # 或 ~/.zshrc"
  fi

  echo ""
  echo "Done! Run 'goldbot' to start."
}

main "$@"
