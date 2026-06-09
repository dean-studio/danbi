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

# rustup toolchain (arm64) 을 항상 우선시. Homebrew 의 cargo 가
# /usr/local/bin (Intel) 에 깔려 있으면 host triple 이 x86_64 로 잡혀
# Intel 바이너리가 빠지는 사고가 났던 적 있음 (v0.3.0). 다음 릴리즈에
# 같은 함정에 빠지지 않도록 PATH 우선순위 + 명시적 --target.
export PATH="$HOME/.cargo/bin:$PATH"

KEY_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/.tauri/danbi.key}"
ROOT="$(pwd)"
VERSION="$(node -p "require('./package.json').version")"
OUT="$ROOT/release/v$VERSION"
# --target 을 박으면 산출물 경로에 target triple 한 단 더 들어감.
TARGET_TRIPLE="aarch64-apple-darwin"
BUNDLE="$ROOT/src-tauri/target/$TARGET_TRIPLE/release/bundle"

# productName 이 바뀌어도 글롭으로 잡히도록 패턴화. 현재는 "단비".
DMG_GLOB="$BUNDLE/dmg/*_${VERSION}_aarch64.dmg"
TARGZ_GLOB="$BUNDLE/macos/*.app.tar.gz"
SIG_GLOB="$BUNDLE/macos/*.app.tar.gz.sig"

DST_DMG="$OUT/Danbi_${VERSION}_aarch64.dmg"
DST_TARGZ="$OUT/Danbi.app.tar.gz"
DST_SIG="$OUT/Danbi.app.tar.gz.sig"
DST_JSON="$OUT/latest.json"

NOTARIZE_PROFILE="${NOTARIZE_PROFILE:-danbi-notarize}"

if [[ "${1:-}" != "--skip-build" ]]; then
  if [[ ! -f "$KEY_PATH" ]]; then
    echo "✗ signing key not found at $KEY_PATH" >&2
    echo "  generate with: npx tauri signer generate --password \"\" --write-keys $KEY_PATH" >&2
    exit 1
  fi

  echo "→ building signed release v$VERSION (target=$TARGET_TRIPLE)"
  echo "  using cargo: $(which cargo)"
  echo "  host triple: $(rustc -vV | awk '/^host:/ {print $2}')"
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_PATH")" \
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
    npx tauri build --bundles app,dmg --target "$TARGET_TRIPLE"

  APP_PATH="$BUNDLE/macos/단비.app"
  DMG_PATH=$(ls $BUNDLE/dmg/*_${VERSION}_aarch64.dmg 2>/dev/null | head -1)

  # Sanity check: built binary must actually be arm64. v0.3.0 went out as
  # x86_64 by accident — guard so it never repeats.
  BIN_PATH="$APP_PATH/Contents/MacOS/danbi"
  ARCH_DESC="$(file "$BIN_PATH" 2>/dev/null || true)"
  if ! grep -q "arm64" <<<"$ARCH_DESC"; then
    echo "✗ built binary is NOT arm64 — got: $ARCH_DESC" >&2
    echo "  this would trigger the 'fallback platforms not found' updater bug." >&2
    exit 1
  fi
  echo "  ✓ verified arm64: $(basename "$BIN_PATH")"

  if ! xcrun notarytool history --keychain-profile "$NOTARIZE_PROFILE" >/dev/null 2>&1; then
    echo "⚠ notarize profile '$NOTARIZE_PROFILE' not found in keychain — skipping notarization." >&2
    echo "  set up with: xcrun notarytool store-credentials \"$NOTARIZE_PROFILE\" --apple-id ... --team-id ... --password ..." >&2
  else
    echo "→ notarizing .app (Apple servers, may take 1-5 min)"
    APP_ZIP="$BUNDLE/macos/단비.app.zip"
    /usr/bin/ditto -c -k --keepParent "$APP_PATH" "$APP_ZIP"
    xcrun notarytool submit "$APP_ZIP" --keychain-profile "$NOTARIZE_PROFILE" --wait
    rm -f "$APP_ZIP"
    echo "→ stapling .app"
    xcrun stapler staple "$APP_PATH"

    echo "→ re-creating .app.tar.gz from stapled .app + re-signing"
    TARGZ="$BUNDLE/macos/단비.app.tar.gz"
    rm -f "$TARGZ" "$TARGZ.sig"
    /usr/bin/tar -czf "$TARGZ" -C "$BUNDLE/macos" "단비.app"
    npx tauri signer sign \
      --private-key-path "$KEY_PATH" \
      --password "" \
      "$TARGZ"

    if [[ -n "$DMG_PATH" ]]; then
      echo "→ notarizing .dmg"
      xcrun notarytool submit "$DMG_PATH" --keychain-profile "$NOTARIZE_PROFILE" --wait
      echo "→ stapling .dmg"
      xcrun stapler staple "$DMG_PATH"
    fi

    echo "→ verifying Gatekeeper acceptance"
    spctl -a -vvv -t exec "$APP_PATH" 2>&1 | sed 's/^/    /'
  fi
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

# darwin-x86_64 도 같은 url/sig 로 노출 — 0.3.0 사고로 일부 사용자가 Intel
# 빌드를 깔린 상태라 그쪽 updater 가 darwin-x86_64 키만 찾는다.
# .app.tar.gz 안 의 .app 는 실제로 native arm64 라 다운로드받은 사용자는
# 자동으로 native 로 전환됨.
cat > "$DST_JSON" <<EOF
{
  "version": "$VERSION",
  "notes": "v$VERSION 릴리즈",
  "pub_date": "$PUB_DATE",
  "platforms": {
    "darwin-aarch64": {
      "signature": "$SIG_CONTENT",
      "url": "https://github.com/dean-studio/danbi/releases/download/v$VERSION/Danbi.app.tar.gz"
    },
    "darwin-x86_64": {
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
