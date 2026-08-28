#!/bin/sh

set -eu

REPO="${MOONREVIEW_REPO:-antoineMoPa/moon-dev-tools}"
INSTALL_DIR="${MOONREVIEW_INSTALL_DIR:-$HOME/.local/bin}"
# The three executables. They are one program opened on three different things, and they ship
# together, but an archive from before the split holds only the first.
PROGRAMS="moonreview moontasks moonshell"
INSTALL_PATH="${INSTALL_DIR}/moonreview"
DOWNLOAD_BASE_URL="${MOONREVIEW_DOWNLOAD_BASE_URL:-}"

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "moonreview installer: missing required command: $1" >&2
        exit 1
    fi
}

detect_rc_file() {
    shell_name="$(basename "${SHELL:-}")"
    case "$shell_name" in
        zsh)
            echo "$HOME/.zshrc"
            ;;
        bash)
            if [ -f "$HOME/.bashrc" ]; then
                echo "$HOME/.bashrc"
            else
                echo "$HOME/.bash_profile"
            fi
            ;;
        *)
            if [ -f "$HOME/.zshrc" ]; then
                echo "$HOME/.zshrc"
            elif [ -f "$HOME/.bashrc" ]; then
                echo "$HOME/.bashrc"
            else
                echo "$HOME/.profile"
            fi
            ;;
    esac
}

append_path_export() {
    rc_file="$1"
    export_line='export PATH="$HOME/.local/bin:$PATH"'

    if [ "$INSTALL_DIR" != "$HOME/.local/bin" ]; then
        return 1
    fi

    mkdir -p "$(dirname "$rc_file")"
    touch "$rc_file"

    if grep -F "$export_line" "$rc_file" >/dev/null 2>&1; then
        return 0
    fi

    printf '\n%s\n' "$export_line" >>"$rc_file"
    return 2
}

verify_platform() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "${os}:${arch}" in
        Darwin:arm64)
            echo "moonreview-aarch64-apple-darwin"
            ;;
        Linux:x86_64 | Linux:amd64)
            echo "moonreview-x86_64-unknown-linux-gnu"
            ;;
        Linux:aarch64 | Linux:arm64)
            echo "moonreview-aarch64-unknown-linux-gnu"
            ;;
        *)
            echo "moonreview installer supports macOS arm64, Linux amd64, and Linux arm64 only (detected ${os} ${arch})." >&2
            exit 1
            ;;
    esac
}

checksum_cmd() {
    if command -v shasum >/dev/null 2>&1; then
        echo "shasum -a 256"
        return
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        echo "sha256sum"
        return
    fi

    echo ""
}

need_cmd curl
need_cmd tar
need_cmd mktemp

ASSET_BASENAME="$(verify_platform)"
ARCHIVE_NAME="${ASSET_BASENAME}.tar.gz"
CHECKSUM_NAME="${ARCHIVE_NAME}.sha256"

SUM_CMD="$(checksum_cmd)"
if [ -z "$SUM_CMD" ]; then
    echo "moonreview installer: missing checksum tool (shasum or sha256sum)." >&2
    exit 1
fi

TMP_DIR="$(mktemp -d)"
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

if [ -n "$DOWNLOAD_BASE_URL" ]; then
    ARCHIVE_URL="${DOWNLOAD_BASE_URL}/${ARCHIVE_NAME}"
    CHECKSUM_URL="${DOWNLOAD_BASE_URL}/${CHECKSUM_NAME}"
else
    ARCHIVE_URL="https://github.com/${REPO}/releases/latest/download/${ARCHIVE_NAME}"
    CHECKSUM_URL="https://github.com/${REPO}/releases/latest/download/${CHECKSUM_NAME}"
fi

echo "Downloading moonreview from ${REPO}..."
curl -fsSL "$ARCHIVE_URL" -o "${TMP_DIR}/${ARCHIVE_NAME}"
curl -fsSL "$CHECKSUM_URL" -o "${TMP_DIR}/${CHECKSUM_NAME}"

(
    cd "$TMP_DIR"
    $SUM_CMD -c "$CHECKSUM_NAME"
)

mkdir -p "$INSTALL_DIR"
tar -xzf "${TMP_DIR}/${ARCHIVE_NAME}" -C "$TMP_DIR"

if [ ! -f "${TMP_DIR}/moonreview" ]; then
    echo "moonreview installer: the archive does not contain moonreview." >&2
    exit 1
fi

installed=""
for program in $PROGRAMS; do
    if [ -f "${TMP_DIR}/${program}" ]; then
        install -m 0755 "${TMP_DIR}/${program}" "${INSTALL_DIR}/${program}"
        installed="${installed} ${program}"
    fi
done

path_updated="no"
current_shell_hint=""

case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        ;;
    *)
        rc_file="$(detect_rc_file)"
        if append_path_export "$rc_file"; then
            :
        else
            status="$?"
            if [ "$status" -eq 2 ]; then
                path_updated="yes"
            fi
        fi
        current_shell_hint="export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
esac

echo "Installed${installed} to ${INSTALL_DIR}"
"$INSTALL_PATH" --help >/dev/null 2>&1 || true

# The executables are on PATH now; this is what puts them in Spotlight, Launchpad or an
# application menu as well. A platform with nowhere to write a launcher refuses here, which is
# not a failed install.
if "$INSTALL_PATH" install-launchers >/dev/null 2>&1; then
    echo "Installed desktop launchers"
fi

if [ "$path_updated" = "yes" ]; then
    echo "Added $INSTALL_DIR to PATH in $(detect_rc_file)"
fi

if [ -n "$current_shell_hint" ]; then
    echo "For this shell, run:"
    echo "  ${current_shell_hint}"
fi

echo "Run: moonreview   — a review of the repo"
for program in $installed; do
    case "$program" in
        moontasks) echo "     moontasks    — the task board" ;;
        moonshell) echo "     moonshell    — a shell in the repo" ;;
    esac
done
