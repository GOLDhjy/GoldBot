#!/usr/bin/env bash
set -euo pipefail

GITHUB_REPO="${GOLDBOT_GITHUB_REPO:-GOLDhjy/GoldBot}"
REPO_URL="${GOLDBOT_REPO_URL:-https://github.com/GOLDhjy/GoldBot.git}"
VERSION="${GOLDBOT_VERSION:-latest}"
MODE="binary"
BIN_NAME="goldbot"
BIN_DIR="${INSTALL_BIN_DIR:-$HOME/.local/bin}"
TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/goldbot-target}"
SCRIPT_SOURCE="${BASH_SOURCE[0]-$0}"
ROOT_DIR=""
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

if [[ "$SCRIPT_SOURCE" != "bash" && "$SCRIPT_SOURCE" != "-" ]]; then
  ROOT_DIR="$(cd "$(dirname "$SCRIPT_SOURCE")/.." && pwd 2>/dev/null || true)"
fi

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

detect_suffix() {
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
    MINGW*|MSYS*|CYGWIN*)
      echo "windows-x86_64"
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

resolve_tag() {
  local tag="$VERSION"
  if [[ "$tag" == "latest" ]]; then
    tag="$(latest_tag)"
  fi
  if [[ -z "$tag" || "$tag" != v* ]]; then
    echo "Failed to resolve release tag. Got: ${tag:-<empty>}"
    return 1
  fi
  printf '%s\n' "$tag"
}

download_file() {
  local url="$1"
  local out="$2"
  curl -fL --retry 3 --retry-delay 1 -o "$out" "$url"
}

print_success() {
  echo
  echo "Installed to: $BIN_DIR/$BIN_NAME"
  echo
  echo "If needed, add this to your shell rc:"
  echo "  export PATH=\"$BIN_DIR:\$PATH\""
  echo
  echo "Done! Run '$BIN_NAME' to start."
}

install_from_binary() {
  require_cmd curl
  require_cmd tar

  local suffix tag asset base_url tmp_dir asset_file found_bin
  suffix="$(detect_suffix)" || {
    echo "Unsupported platform for prebuilt binary."
    echo "Supported: macOS (x86_64/aarch64), Linux (x86_64), Windows (x86_64)."
    return 1
  }

  if [[ "$suffix" == windows-* ]]; then
    echo "Detected Windows shell. Please use PowerShell installer instead:"
    echo "  irm \"https://raw.githubusercontent.com/$GITHUB_REPO/master/scripts/install.ps1\" | iex"
    return 1
  fi

  tag="$(resolve_tag)" || return 1
  asset="${BIN_NAME}-${tag}-${suffix}.tar.gz"
  base_url="https://github.com/$GITHUB_REPO/releases/download/$tag"

  tmp_dir="$(mktemp -d)"
  register_tmp_dir "$tmp_dir"
  asset_file="$tmp_dir/$asset"

  echo "Downloading release ${tag} (${suffix})..."
  download_file "$base_url/$asset" "$asset_file"

  mkdir -p "$BIN_DIR"
  tar -xzf "$asset_file" -C "$tmp_dir"
  found_bin="$(find "$tmp_dir" -type f -name "$BIN_NAME" | head -n1 || true)"
  if [[ -z "$found_bin" ]]; then
    echo "Binary '$BIN_NAME' not found in archive: $asset"
    return 1
  fi

  cp "$found_bin" "$BIN_DIR/$BIN_NAME"
  chmod +x "$BIN_DIR/$BIN_NAME"
  print_success
}

install_from_source() {
  require_cmd cargo
  require_cmd git

  local source_root tmp_dir tag built_bin
  if [[ -n "$ROOT_DIR" && -f "$ROOT_DIR/Cargo.toml" ]]; then
    source_root="$ROOT_DIR"
  else
    tag="$(resolve_tag)" || return 1
    tmp_dir="$(mktemp -d)"
    register_tmp_dir "$tmp_dir"
    echo "Cloning repository: $REPO_URL ($tag)"
    git clone --depth 1 --branch "$tag" "$REPO_URL" "$tmp_dir/GoldBot"
    source_root="$tmp_dir/GoldBot"
  fi

  echo "Building $BIN_NAME from source..."
  CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$TARGET_DIR" \
    cargo build --release --manifest-path "$source_root/Cargo.toml"

  if [[ -f "$TARGET_DIR/release/$BIN_NAME" ]]; then
    built_bin="$TARGET_DIR/release/$BIN_NAME"
  elif [[ -f "$TARGET_DIR/release/GoldBot" ]]; then
    built_bin="$TARGET_DIR/release/GoldBot"
  else
    echo "Build succeeded but binary not found in $TARGET_DIR/release"
    return 1
  fi

  mkdir -p "$BIN_DIR"
  cp "$built_bin" "$BIN_DIR/$BIN_NAME"
  chmod +x "$BIN_DIR/$BIN_NAME"
  print_success
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --source)
      MODE="source"
      shift
      ;;
    --repo)
      REPO_URL="${2:-$REPO_URL}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1"
      usage
      exit 1
      ;;
  esac
done

if [[ "$MODE" == "source" ]]; then
  install_from_source
else
  install_from_binary || {
    echo
    echo "Binary install failed. You can retry source build mode:"
    echo "  bash scripts/install.sh --source"
    exit 1
  }
fi
