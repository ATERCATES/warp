#!/usr/bin/env bash
set +e

_present() { command -v "$1" >/dev/null 2>&1; }

_find_tmux() {
    if _present tmux; then echo "tmux"
    elif [ -x "$HOME/.warp/tmux/execute_tmux.sh" ]; then echo "$HOME/.warp/tmux/execute_tmux.sh"
    else echo ""
    fi
}

_tmux_supported() {
    case "$1" in
        ""|1.*|2.*|3.0|3.0.*|3.1|3.1.*) echo "false" ;;
        *) echo "true" ;;
    esac
}

_passthrough_configured() {
    for f in "$HOME/.tmux.conf" "$HOME/.config/tmux/tmux.conf"; do
        [ -f "$f" ] || continue
        if grep -Eq '^[[:space:]]*set(-option)?[[:space:]]+(-g[[:space:]]+)?allow-passthrough[[:space:]]+on' "$f"; then
            echo "true"
            return
        fi
    done
    echo "false"
}

_detect_pkg() {
    case "$1" in
        Darwin) _present brew && echo "brew" ;;
        Linux)
            for p in apt-get dnf yum pacman zypper apk; do
                _present "$p" && { echo "$p"; return; }
            done
            ;;
    esac
}

_root_access() {
    if [ "$(id -u)" = "0" ]; then echo "is_root"
    elif _present sudo && sudo -n true 2>/dev/null; then echo "can_run_sudo"
    else echo "no_root_access"
    fi
}

_escape_json() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

TMUX_BIN=$(_find_tmux)
TMUX_VERSION=""
if [ -n "$TMUX_BIN" ]; then
    TMUX_VERSION=$("$TMUX_BIN" -V 2>/dev/null | awk '{print $2}')
fi
TMUX_SUPPORTED=$(_tmux_supported "$TMUX_VERSION")
PASSTHROUGH=$(_passthrough_configured)
SHELL_INTEGRATION="false"
[ -d "$HOME/.warp/shell_integration" ] && SHELL_INTEGRATION="true"
OS=$(uname -s)
PKG=$(_detect_pkg "$OS")
ROOT_ACCESS=$(_root_access)

printf '__WARP_REMOTE_SESSIONS_PROBE_BEGIN__\n'
printf '{"tmux_bin":"%s","tmux_version":"%s","tmux_supported":%s,"passthrough":%s,"shell_integration":%s,"os":"%s","pkg":"%s","root_access":"%s"}\n' \
    "$(_escape_json "$TMUX_BIN")" \
    "$(_escape_json "$TMUX_VERSION")" \
    "$TMUX_SUPPORTED" \
    "$PASSTHROUGH" \
    "$SHELL_INTEGRATION" \
    "$(_escape_json "$OS")" \
    "$(_escape_json "$PKG")" \
    "$ROOT_ACCESS"
printf '__WARP_REMOTE_SESSIONS_PROBE_END__\n'
