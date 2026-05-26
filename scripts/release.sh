#!/usr/bin/env bash
# Builds a signed macOS release and emits ASCII-named artifacts ready to
# upload to GitHub Releases.
#
# Why ASCII rename: productName 은 "단비" 그대로 두지만 (윈도우 타이틀·About
# 표시는 한글 유지), 빌드 결과물 파일명도 "단비_x.y.z_aarch64.dmg" 처럼
# 한글이 들어감. 한글 URL 은 GitHub Releases CDN/updater 다운로드에서
# percent-encoding 이슈를 종종 일으켜서, 릴리즈 페이로드만 ASCII 로
# rename 한다.
#
# Usage:
#   scripts/release.sh                # 전체 (build + rename + latest.json)
#   scripts/release.sh --skip-build   # 이미 빌드된 산출물만 rename + json

set -euo pipefail

cd "$(dirname "$0")/.."

KEY_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/.tauri/danbi.key}"
ROOT="$(pwd)"
VERSION="$(node -p "require('./package.json').version")"
OUT="$ROOT/release/v$VERSION"
BUNDLE="$ROOT/src-tauri/target/release/bundle"

# productName 이 바뀌어도 글롭으로 잡히도록 패턴화. 현재는 "단비".
DMG_GLOB="$BUNDLE/dmg/*_${VERSION}_aarch64.dmg"
TARGZ_GLOB="$BUNDLE/macos/*.app.tar.gz"
SIG_GLOB="$BUNDLE/macos/*.app.tar.gz.sig"

DST_DMG="$OUT/Danbi_${VERSION}_aarch64.dmg"
DST_TARGZ="$OUT/Danbi.app.tar.gz"
DST_SIG="$OUT/Danbi.app.tar.gz.sig"
DST_JSON="$OUT/latest.json"

if [[ "${1:-}" != "--skip-build" ]]; then
  if [[ ! -f "$KEY_PATH" ]]; then
    echo "✗ signing key not found at $KEY_PATH" >&2
    echo "  generate with: npx tauri signer generate --password \"\" --write-keys $KEY_PATH" >&2
    exit 1
  fi

  echo "→ building signed release v$VERSION"
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_PATH")" \
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
    npx tauri build --bundles app,dmg
fi

mkdir -p "$OUT"

DMG_SRC=$(ls $DMG_GLOB 2>/dev/null | head -1 || true)
TARGZ_SRC=$(ls $TARGZ_GLOB 2>/dev/null | head -1 || true)
SIG_SRC=$(ls $SIG_GLOB 2>/dev/null | head -1 || true)

if [[ -z "$DMG_SRC" || -z "$TARGZ_SRC" || -z "$SIG_SRC" ]]; then
  echo "✗ expected artifacts missing under $BUNDLE" >&2
  echo "  dmg=$DMG_SRC" >&2
  echo "  targz=$TARGZ_SRC" >&2
  echo "  sig=$SIG_SRC" >&2
  exit 1
fi

echo "→ ASCII rename → $OUT"
cp "$DMG_SRC"   "$DST_DMG"
cp "$TARGZ_SRC" "$DST_TARGZ"
cp "$SIG_SRC"   "$DST_SIG"

SIG_CONTENT="$(cat "$DST_SIG")"
PUB_DATE="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

cat > "$DST_JSON" <<EOF
{
  "version": "$VERSION",
  "notes": "v$VERSION 릴리즈",
  "pub_date": "$PUB_DATE",
  "platforms": {
    "darwin-aarch64": {
      "signature": "$SIG_CONTENT",
      "url": "https://github.com/dean-studio/danbi/releases/download/v$VERSION/Danbi.app.tar.gz"
    }
  }
}
EOF

echo
echo "✓ release artifacts ready under $OUT"
ls -la "$OUT"
echo
echo "next:"
echo "  git tag -a v$VERSION -m \"v$VERSION\" && git push origin v$VERSION"
echo "  gh release create v$VERSION \\"
echo "    \"$DST_DMG\" \"$DST_TARGZ\" \"$DST_SIG\" \"$DST_JSON\" \\"
echo "    --title \"v$VERSION\" --notes-file CHANGELOG.md"
