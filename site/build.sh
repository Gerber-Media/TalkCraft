#!/usr/bin/env bash
# Builds the product pages https://talkcraft.gerber.media (and /anleitung/, the tutorial) into
# site/out: fills in the newest GitHub release's version and download links and the shared footer,
# copies icons, styles, the README animations and the tutorial pictures.
#
#   site/build.sh            build into site/out
#   site/build.sh --publish  build and push site/out to the `pages` branch of the Codefloe repo
#                            GerberMedia/talkcraft-site (served by Codefloe Pages)
#
# Needs curl and python3. GH_TOKEN is optional (only raises GitHub's rate limit).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
OUT="$HERE/out"
REPO="Gerber-Media/TalkCraft"
PAGES_REMOTE="https://GerberMedia@codefloe.com/GerberMedia/talkcraft-site.git"

auth=()
if [ -n "${GH_TOKEN:-}" ]; then auth=(-H "Authorization: Bearer $GH_TOKEN"); fi
release="$(curl -fsSL ${auth[@]+"${auth[@]}"} "https://api.github.com/repos/$REPO/releases/latest")"

rm -rf "$OUT"
mkdir -p "$OUT/media"
/bin/cp -f "$HERE"/favicon.ico "$HERE"/favicon-32.png "$HERE"/apple-touch-icon.png "$HERE"/icon-256.png "$OUT/"
/bin/cp -f "$HERE"/install.sh "$HERE"/install.ps1 "$HERE"/style.css "$HERE"/site.js "$OUT/"
mkdir -p "$OUT/anleitung" "$OUT/media/tutorial"
# The tutorial's pictures, made by site/tutorial/make_media.sh.
/bin/cp -f "$HERE"/media/tutorial/* "$OUT/media/tutorial/"
/bin/cp -f "$ROOT"/docs/media/talkcraft-{hero,record,edit,publish}.gif "$OUT/media/"
# Codefloe Pages serves the domain set in the repo settings; this file only documents it.
echo "talkcraft.gerber.media" > "$OUT/CNAME"

RELEASE_JSON="$release" python3 - "$HERE" "$OUT" <<'PY'
import json, os, re, sys
rel = json.loads(os.environ["RELEASE_JSON"])
assets = {a["name"]: a["browser_download_url"] for a in rel.get("assets", [])}
version = rel["tag_name"].lstrip("v")

def pick(pattern):
    for name, url in assets.items():
        if re.search(pattern, name):
            return name, url
    sys.exit(f"release {rel['tag_name']} has no file matching {pattern}")

values = {"VERSION": version}
for key, pattern in {
    "DMG": r"^talkcraft-[\d.]+-macos-universal\.dmg$",
    "MSI": r"^talkcraft-[\d.]+-windows-x64\.msi$",
    "ZIP": r"^talkcraft-[\d.]+-windows-x64-portable\.zip$",
    "APPIMAGE": r"^talkcraft-[\d.]+-linux-x86_64\.AppImage$",
    "DEB": r"\.deb$",
    "RPM": r"\.rpm$",
    "TGZ": r"^talkcraft-[\d.]+-linux-x86_64\.tar\.gz$",
    "SUMS": r"^SHA256SUMS\.txt$",
}.items():
    name, url = pick(pattern)
    values["FILE_" + key] = name
    values["URL_" + key] = url

src, out = sys.argv[1], sys.argv[2]
# The footer (Impressum, privacy) is the same on every page.
values["FOOTER"] = open(os.path.join(src, "footer.html"), encoding="utf-8").read().strip()
for page in ["index.html", "anleitung/index.html"]:
    html = open(os.path.join(src, page), encoding="utf-8").read()
    html = re.sub(r"\{\{(\w+)\}\}", lambda m: values[m.group(1)], html)
    if "{{" in html:
        sys.exit(f"unfilled placeholder left in {page}")
    open(os.path.join(out, page), "w", encoding="utf-8").write(html)
print(f"site/out for TalkCraft {version}")
PY

if [ "${1:-}" = "--publish" ]; then
  work="$(mktemp -d)"
  git clone -q --depth 1 --branch pages "$PAGES_REMOTE" "$work" 2>/dev/null || { git -C "$work" init -q && git -C "$work" checkout -q -b pages && git -C "$work" remote add origin "$PAGES_REMOTE"; }
  find "$work" -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
  /bin/cp -Rf "$OUT"/. "$work/"
  git -C "$work" add -A
  if git -C "$work" diff --cached --quiet; then
    echo "nothing changed"
  else
    git -C "$work" -c user.email=hello@gerber.media -c user.name="Dirk Gerber" commit -q -m "TalkCraft-Seite aktualisiert"
    git -C "$work" push -q origin pages
    echo "published"
  fi
  rm -rf "$work"
fi
