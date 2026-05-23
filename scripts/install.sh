#!/usr/bin/env bash
# Install the latest `warp-oss` build from ATERCATES/warp.
#
# Detects host OS / arch, fetches the matching artifact from the most recent
# `fork-v*` GitHub Release, and installs it:
#   - macOS Intel → ~/Applications/WarpOss.app  (Gatekeeper quarantine cleared)
#   - Debian-like → apt install of the .deb       (preferred when dpkg present)
#   - Other Linux → ~/.local/bin/warp-oss        + .desktop entry
#
# Usage:
#   curl -sSf https://raw.githubusercontent.com/ATERCATES/warp/master/scripts/install.sh | bash
#   bash scripts/install.sh --tag fork-v0.2.0
#   bash scripts/install.sh --uninstall
#
# Env vars:
#   WARP_FORK_REPO        — override the source repo (default ATERCATES/warp)
#   WARP_FORCE_APPIMAGE=1 — on Linux, skip the .deb path and install the AppImage

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
      sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
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
or build locally: cargo build --release --bin warp-oss"
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

# On Linux, prefer the .deb if dpkg + apt are available — gives users proper
# package management. Set WARP_FORCE_APPIMAGE=1 to override.
linux_install_method() {
  if [[ "${WARP_FORCE_APPIMAGE:-0}" == "1" ]]; then
    echo "appimage"
    return
  fi
  if command -v dpkg >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1; then
    echo "deb"
  else
    echo "appimage"
  fi
}

resolve_tag() {
  if [[ -n "$TAG" ]]; then
    echo "$TAG"
    return
  fi
  local api="https://api.github.com/repos/${REPO}/releases"
  local latest
  latest=$(curl -fsSL "$api" \
    | grep -oE '"tag_name": *"fork-v[^"]+"' \
    | head -1 \
    | sed -E 's/.*"(fork-v[^"]+)".*/\1/')
  [[ -n "$latest" ]] || fail "No fork-v* release found in $REPO. Pass --tag <tag> or wait for one to be published."
  echo "$latest"
}

# Look up the actual .deb asset filename from the GitHub Release API (it
# embeds the version derived from the release tag at build time, so we can't
# hard-code it).
deb_asset_name() {
  local tag="$1"
  curl -fsSL "https://api.github.com/repos/${REPO}/releases/tags/${tag}" \
    | grep -oE '"name": *"warp-terminal-oss[^"]+\.deb"' \
    | head -1 \
    | sed -E 's/.*"(warp-terminal-oss[^"]+\.deb)".*/\1/'
}

download() {
  local url="$1" out="$2"
  step "Downloading $(basename "$out")"
  curl -fL --progress-bar -o "$out" "$url"
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

install_linux_deb() {
  local deb="$1"
  step "Installing .deb via apt"
  # Prefer apt (resolves dependencies); fall back to plain dpkg.
  sudo apt-get install -y "$deb" || sudo dpkg -i "$deb"
  cat <<EOF

✓ Installed warp-terminal-oss via dpkg.
  Launch from your app menu, or run:
      warp-oss
EOF
}

install_linux_appimage() {
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

  if "$target_bin" --appimage-extract '*.png' >/dev/null 2>&1; then
    local extracted_icon
    extracted_icon=$(find squashfs-root -name '*.png' -printf '%s %p\n' 2>/dev/null \
      | sort -rn | head -1 | awk '{print $2}' || true)
    if [[ -n "${extracted_icon:-}" && -f "$extracted_icon" ]]; then
      cp "$extracted_icon" "$icons_dir/warp-oss.png"
    fi
    rm -rf squashfs-root
  fi

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
      if command -v dpkg >/dev/null 2>&1 && dpkg -s warp-terminal-oss >/dev/null 2>&1; then
        sudo apt-get remove -y warp-terminal-oss
        note "Removed warp-terminal-oss via apt"
      fi
      rm -f "${HOME}/.local/bin/warp-oss"
      rm -f "${HOME}/.local/share/applications/warp-oss.desktop"
      rm -f "${HOME}/.local/share/icons/hicolor/256x256/apps/warp-oss.png"
      if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "${HOME}/.local/share/applications" 2>/dev/null || true
      fi
      note "Removed AppImage install (binary, desktop entry, icon)"
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

  local platform tag base_url tmp method
  platform=$(detect_platform)
  tag=$(resolve_tag)
  base_url="https://github.com/${REPO}/releases/download/${tag}"
  tmp=$(mktemp -d)
  trap "rm -rf $tmp" EXIT

  if [[ "$platform" == "linux-x86_64" ]]; then
    method=$(linux_install_method)
  else
    method=default
  fi
  step "Resolved release: $tag (platform: $platform, method: $method)"

  case "$method" in
    deb)
      local deb_name
      deb_name=$(deb_asset_name "$tag")
      [[ -n "$deb_name" ]] || fail "Release $tag has no .deb asset. Force AppImage via WARP_FORCE_APPIMAGE=1, or pass --tag pointing at a release that includes one."
      download "$base_url/$deb_name" "$tmp/$deb_name"
      install_linux_deb "$tmp/$deb_name"
      ;;
    *)
      case "$platform" in
        macos-x86_64)
          download "$base_url/WarpOss-x86_64-apple-darwin.zip" "$tmp/macos.zip"
          install_macos "$tmp/macos.zip"
          ;;
        linux-x86_64)
          download "$base_url/WarpOss-x86_64-linux.AppImage" "$tmp/warp-oss.AppImage"
          install_linux_appimage "$tmp/warp-oss.AppImage"
          ;;
      esac
      ;;
  esac
}

main "$@"
