#!/usr/bin/env bash
# Install the latest `warp-oss` build from ATERCATES/warp.
#
# Detects host OS / arch, fetches the matching artifact from the most recent
# `fork-v*` GitHub Release, and installs it:
#   - macOS  → ~/Applications/WarpOss.app  (Gatekeeper quarantine cleared)
#   - Linux  → ~/.local/bin/warp-oss       + .desktop entry under ~/.local/share/applications
#
# Usage:
#   curl -sSf https://raw.githubusercontent.com/ATERCATES/warp/master/scripts/install.sh | bash
#   bash scripts/install.sh --tag fork-v0.2.0
#   bash scripts/install.sh --uninstall

set -euo pipefail

REPO="${WARP_FORK_REPO:-ATERCATES/warp}"
TAG=""
ACTION="install"

while (( "$#" )); do
  case "$1" in
    --tag) TAG="$2"; shift 2 ;;
    --tag=*) TAG="${1#*=}"; shift ;;
    --uninstall) ACTION="uninstall"; shift ;;
    -h|--help)
      sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done

note()  { printf '  %s\n' "$*"; }
step()  { printf '\n→ %s\n' "$*"; }
fail()  { printf '\n✗ %s\n' "$*" >&2; exit 1; }

detect_platform() {
  local uname_s uname_m
  uname_s="$(uname -s)"
  uname_m="$(uname -m)"
  case "$uname_s" in
    Darwin)
      case "$uname_m" in
        x86_64) echo "macos-x86_64" ;;
        arm64|aarch64)
          fail "macOS arm64 is not published by this fork. Run on an Intel Mac, \
or build locally: cargo build --release --bin warp-oss --features warp/remote_sessions"
          ;;
        *) fail "Unsupported macOS arch: $uname_m" ;;
      esac
      ;;
    Linux)
      case "$uname_m" in
        x86_64) echo "linux-x86_64" ;;
        *) fail "Unsupported Linux arch: $uname_m (only x86_64 is published)" ;;
      esac
      ;;
    *) fail "Unsupported OS: $uname_s" ;;
  esac
}

artifact_for_platform() {
  case "$1" in
    macos-x86_64) echo "WarpOss-x86_64-apple-darwin.zip" ;;
    linux-x86_64) echo "WarpOss-x86_64-linux.AppImage" ;;
  esac
}

resolve_tag() {
  if [[ -n "$TAG" ]]; then
    echo "$TAG"
    return
  fi
  # Pick the latest fork-v* release without requiring `gh` or auth.
  local api="https://api.github.com/repos/${REPO}/releases"
  local latest
  latest=$(curl -fsSL "$api" \
    | grep -oE '"tag_name": *"fork-v[^"]+"' \
    | head -1 \
    | sed -E 's/.*"(fork-v[^"]+)".*/\1/')
  [[ -n "$latest" ]] || fail "No fork-v* release found in $REPO. Pass --tag <tag> or wait for one to be published."
  echo "$latest"
}

download() {
  local url="$1" out="$2"
  step "Downloading $(basename "$out")"
  curl -fL --progress-bar -o "$out" "$url"
}

verify_checksum() {
  local artifact="$1" checksum_file="$2"
  step "Verifying checksum"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$(dirname "$artifact")" && sha256sum -c "$(basename "$checksum_file")")
  else
    # macOS uses shasum
    local expected actual
    expected=$(awk '{print $1}' "$checksum_file")
    actual=$(shasum -a 256 "$artifact" | awk '{print $1}')
    [[ "$expected" == "$actual" ]] || fail "Checksum mismatch (expected $expected, got $actual)"
  fi
}

install_macos() {
  local zip_path="$1"
  local app_dir="${HOME}/Applications"
  local target="$app_dir/WarpOss.app"

  step "Installing to $target"
  mkdir -p "$app_dir"
  if [[ -d "$target" ]]; then
    note "Removing existing $target"
    rm -rf "$target"
  fi
  ditto -x -k "$zip_path" "$app_dir"
  [[ -d "$target" ]] || fail "Extraction produced no $target"

  step "Clearing Gatekeeper quarantine (unsigned build)"
  xattr -dr com.apple.quarantine "$target" 2>/dev/null || true

  cat <<EOF

✓ Installed WarpOss.app to $app_dir
  Launch from Finder, Spotlight, or:
      open "$target"
EOF
}

install_linux() {
  local appimage="$1"
  local bin_dir="${HOME}/.local/bin"
  local apps_dir="${HOME}/.local/share/applications"
  local icons_dir="${HOME}/.local/share/icons/hicolor/256x256/apps"
  local target_bin="$bin_dir/warp-oss"

  step "Installing AppImage to $target_bin"
  mkdir -p "$bin_dir"
  install -m 0755 "$appimage" "$target_bin"

  step "Creating desktop entry"
  mkdir -p "$apps_dir" "$icons_dir"

  # Best-effort icon extraction from the AppImage.
  local extract_dir
  extract_dir=$(mktemp -d)
  if "$target_bin" --appimage-extract '*.png' >/dev/null 2>&1; then
    # Pick the largest PNG.
    local extracted_icon
    extracted_icon=$(find squashfs-root -name '*.png' -printf '%s %p\n' 2>/dev/null \
      | sort -rn | head -1 | awk '{print $2}' || true)
    if [[ -n "${extracted_icon:-}" && -f "$extracted_icon" ]]; then
      cp "$extracted_icon" "$icons_dir/warp-oss.png"
    fi
    rm -rf squashfs-root
  fi
  rm -rf "$extract_dir"

  cat > "$apps_dir/warp-oss.desktop" <<EOF
[Desktop Entry]
Name=Warp (fork)
Comment=ATERCATES/warp build with gh-9416 remote sessions
Exec=$target_bin %U
Icon=warp-oss
Type=Application
Categories=System;TerminalEmulator;
StartupWMClass=warp-oss
EOF
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$apps_dir" 2>/dev/null || true
  fi

  if [[ ":$PATH:" != *":$bin_dir:"* ]]; then
    cat <<EOF

⚠  $bin_dir is not in your PATH. Add this to your shell rc:
      export PATH="\$HOME/.local/bin:\$PATH"
EOF
  fi

  cat <<EOF

✓ Installed warp-oss to $target_bin
  Launch from your app menu, or run:
      warp-oss
EOF
}

uninstall() {
  step "Uninstalling warp-oss"
  case "$(uname -s)" in
    Darwin)
      rm -rf "${HOME}/Applications/WarpOss.app"
      note "Removed ~/Applications/WarpOss.app"
      ;;
    Linux)
      rm -f "${HOME}/.local/bin/warp-oss"
      rm -f "${HOME}/.local/share/applications/warp-oss.desktop"
      rm -f "${HOME}/.local/share/icons/hicolor/256x256/apps/warp-oss.png"
      if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "${HOME}/.local/share/applications" 2>/dev/null || true
      fi
      note "Removed warp-oss binary, desktop entry, and icon"
      ;;
    *) fail "Unsupported OS: $(uname -s)" ;;
  esac
  echo "✓ Uninstall complete"
}

main() {
  if [[ "$ACTION" == "uninstall" ]]; then
    uninstall
    return
  fi

  local platform artifact tag base_url tmp
  platform=$(detect_platform)
  artifact=$(artifact_for_platform "$platform")
  tag=$(resolve_tag)
  base_url="https://github.com/${REPO}/releases/download/${tag}"
  tmp=$(mktemp -d)
  trap "rm -rf $tmp" EXIT

  step "Resolved release: $tag (platform: $platform)"
  download "$base_url/$artifact"            "$tmp/$artifact"
  download "$base_url/$artifact.sha256"     "$tmp/$artifact.sha256"
  verify_checksum "$tmp/$artifact" "$tmp/$artifact.sha256"

  case "$platform" in
    macos-*) install_macos "$tmp/$artifact" ;;
    linux-*) install_linux "$tmp/$artifact" ;;
  esac
}

main "$@"
