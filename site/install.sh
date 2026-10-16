#!/bin/sh
# Installs the newest TalkCraft release on macOS or Linux:
#
#   curl -fsSL https://talkcraft.gerber.media/install.sh | sh
#
# Downloads the release from GitHub, checks it against SHA256SUMS.txt and installs it:
#   macOS  TalkCraft.app into /Applications (or ~/Applications without write access there;
#          TALKCRAFT_APPS=<folder> picks another one)
#   Linux  the AppImage as ~/.local/bin/talkcraft, plus a menu entry
# Files fetched with curl carry no quarantine flag, so Gatekeeper doesn't block the first start.
set -eu

REPO="Gerber-Media/TalkCraft"
SITE="https://talkcraft.gerber.media"

case "${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}" in de*) de=1 ;; *) de=0 ;; esac
# Usage: say <German> <English>
say() { if [ "$de" = 1 ]; then printf '%s\n' "$1"; else printf '%s\n' "$2"; fi; }
die() { say "TalkCraft: $1" "TalkCraft: $2" >&2; exit 1; }

sha256() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1; else sha256sum "$1" | cut -d' ' -f1; fi
}

# Usage: fetch <file name> <target dir>; downloads one release file and checks its checksum.
fetch() {
  say "Lade $1 …" "Downloading $1 …"
  curl -fL --progress-bar -o "$2/$1" "$BASE/$1" || die "Download fehlgeschlagen." "download failed."
  want="$(grep " \*\{0,1\}$1\$" "$2/SHA256SUMS.txt" | cut -d' ' -f1)"
  [ -n "$want" ] || die "$1 fehlt in SHA256SUMS.txt." "$1 is missing from SHA256SUMS.txt."
  [ "$(sha256 "$2/$1")" = "$want" ] || die "Prüfsumme stimmt nicht, Abbruch." "checksum mismatch, aborting."
  say "Prüfsumme stimmt." "Checksum OK."
}

install_mac() {
  fetch "talkcraft-$VERSION-macos-universal.dmg" "$TMP"
  mkdir -p "$TMP/mnt"
  hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$TMP/mnt" "$TMP/talkcraft-$VERSION-macos-universal.dmg" >/dev/null 2>&1 \
    || die "DMG ließ sich nicht öffnen." "could not open the DMG."
  dest="${TALKCRAFT_APPS:-/Applications}"
  if [ ! -w "$dest" ]; then dest="$HOME/Applications"; fi
  mkdir -p "$dest"
  rm -rf "$dest/TalkCraft.app"
  ditto "$TMP/mnt/TalkCraft.app" "$dest/TalkCraft.app"
  hdiutil detach "$TMP/mnt" >/dev/null 2>&1 || true
  xattr -dr com.apple.quarantine "$dest/TalkCraft.app" 2>/dev/null || true
  say "TalkCraft $VERSION ist installiert: $dest/TalkCraft.app" "TalkCraft $VERSION is installed: $dest/TalkCraft.app"
  say "Starten:  open -a TalkCraft" "Start it: open -a TalkCraft"
}

install_linux() {
  [ "$(uname -m)" = x86_64 ] || die "Es gibt TalkCraft für Linux bisher nur für x86_64." "TalkCraft for Linux is only built for x86_64 so far."
  fetch "talkcraft-$VERSION-linux-x86_64.AppImage" "$TMP"
  bin="$HOME/.local/bin"
  apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
  icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
  mkdir -p "$bin" "$apps" "$icons"
  mv -f "$TMP/talkcraft-$VERSION-linux-x86_64.AppImage" "$bin/talkcraft"
  chmod +x "$bin/talkcraft"
  curl -fsSL -o "$icons/media.gerber.talkcraft.png" "$SITE/icon-256.png" || true
  cat > "$apps/media.gerber.talkcraft.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=TalkCraft
Comment=Podcast studio
Exec=$bin/talkcraft %F
Icon=media.gerber.talkcraft
Categories=AudioVideo;Audio;
Terminal=false
EOF
  say "TalkCraft $VERSION ist installiert: $bin/talkcraft (auch im Anwendungsmenü)" "TalkCraft $VERSION is installed: $bin/talkcraft (also in the applications menu)"
  case ":$PATH:" in *":$bin:"*) ;; *) say "Hinweis: $bin steht nicht im PATH." "Note: $bin is not on your PATH." ;; esac
  say "Startet es nicht, fehlt meist FUSE (z. B. sudo apt install libfuse2)." "If it doesn't start, FUSE is usually missing (e.g. sudo apt install libfuse2)."
}

main() {
  case "$(uname -s)" in
    Darwin) os=mac ;;
    Linux) os=linux ;;
    *) die "Dieses Skript ist für macOS und Linux. Windows: irm $SITE/install.ps1 | iex" \
           "this script is for macOS and Linux. Windows: irm $SITE/install.ps1 | iex" ;;
  esac
  # The newest release's tag, from where github.com/…/releases/latest redirects to.
  tag="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")" \
    || die "GitHub ist nicht erreichbar." "can't reach GitHub."
  tag="${tag##*/}"
  case "$tag" in v[0-9]*) ;; *) die "Keine Version auf GitHub gefunden." "no release found on GitHub." ;; esac
  VERSION="${tag#v}"
  BASE="https://github.com/$REPO/releases/download/$tag"
  say "TalkCraft $VERSION wird installiert." "Installing TalkCraft $VERSION."

  TMP="$(mktemp -d)"
  trap 'hdiutil detach "$TMP/mnt" >/dev/null 2>&1 || true; rm -rf "$TMP"' EXIT
  curl -fsSL -o "$TMP/SHA256SUMS.txt" "$BASE/SHA256SUMS.txt" || die "SHA256SUMS.txt fehlt." "SHA256SUMS.txt is missing."
  "install_$os"
}

# Everything runs from here, so a download cut off halfway through runs nothing.
main "$@"
