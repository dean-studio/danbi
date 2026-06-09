# 릴리즈 자동화

`v*` 태그를 push 하면 `.github/workflows/release.yml` 이 macOS arm64 runner
(`macos-14`) 에서 단비 release 를 빌드/서명/notarize/publish 한다. 로컬
`scripts/release.sh` 와 동일한 흐름이며, 산출물도 동일하다:

- `Danbi_<version>_aarch64.dmg`
- `Danbi.app.tar.gz` + `Danbi.app.tar.gz.sig`
- `latest.json` (darwin-aarch64 + darwin-x86_64 fallback 키)

## 1회만 — Secrets 등록

GitHub repository → Settings → Secrets and variables → Actions, 또는 `gh
secret set` 으로:

| Secret | 값 | 출처 |
| --- | --- | --- |
| `APPLE_CERT_P12_BASE64` | `Developer ID Application` `.p12` 파일을 base64 인코딩한 결과 | Keychain Access → 인증서 우클릭 → Export |
| `APPLE_CERT_PASSWORD` | 위 `.p12` export 시 설정한 password | export 단계에서 본인이 정함 |
| `APPLE_ID` | Apple ID 이메일 (`hckim@dean.kr`) | — |
| `APPLE_TEAM_ID` | 10자리 team id (`663S56834K`) | https://developer.apple.com/account → Membership |
| `APPLE_APP_PWD` | App-specific password (`xxxx-xxxx-xxxx-xxxx`) | https://account.apple.com → 로그인 및 보안 → 앱 암호 |
| `TAURI_SIGNING_PRIVATE_KEY` | `~/.tauri/danbi.key` 파일 내용 그대로 | `cat ~/.tauri/danbi.key` |

### 등록 명령어 모음

```sh
# .p12 export 후 base64 인코딩 (개행 없이)
base64 -i ~/path/to/danbi-developer-id.p12 | pbcopy
gh secret set APPLE_CERT_P12_BASE64   # 클립보드 내용 paste

gh secret set APPLE_CERT_PASSWORD     # .p12 export 시 password
gh secret set APPLE_ID                # 이메일
gh secret set APPLE_TEAM_ID           # 663S56834K
gh secret set APPLE_APP_PWD           # 앱 암호 4단

# tauri ed25519 키는 파일을 그대로
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/danbi.key
```

> `.p12` export 시 password 를 비워두면 일부 macOS keychain 도구가 import 를
> 거부하므로 임의의 password 를 설정해서 base64 + secret 양쪽에 같이 넣어둔다.

## 릴리즈 절차

1. 버전 번호 업데이트 — `package.json`, `src-tauri/Cargo.toml`,
   `src-tauri/tauri.conf.json` 셋 다.
2. `CHANGELOG.md` 의 `[Unreleased]` → `[x.y.z] — YYYY-MM-DD` stamp.
3. commit + push:
   ```sh
   git commit -am "release: vX.Y.Z"
   git push origin main
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin vX.Y.Z
   ```
4. tag push 시점에 GitHub Actions `release` 워크플로우가 자동 실행 — 약
   10–15분 후 release 페이지에 자산이 올라온다.

## 수동 트리거 (이미 publish 된 release 자산 재생성용)

Workflow 의 `workflow_dispatch` 입력으로 tag 만 지정하면 된다:

```sh
gh workflow run release.yml -f tag=v0.5.0
```

워크플로우는 `gh release upload --clobber` 로 자산을 덮어쓰므로 release notes
나 link 는 그대로 유지된다.

## 디버깅

- **codesign 실패** — `.p12` 와 password 가 일치하는지, cert 가 만료 안 됐는지.
- **notarize Rejected** — log URL 이 action output 에 찍힘. 보통
  entitlements/hardened runtime 누락 → `src-tauri/entitlements.plist` 확인.
- **arch verify fail** — runner 가 `macos-14` (arm64) 인지 + cargo target
  install 에 `aarch64-apple-darwin` 이 들어갔는지.
- **release upload 권한 에러** — workflow 의 `permissions: contents: write`
  유지 + repository → Settings → Actions → General → "Read and write
  permissions" 활성화.

## 로컬 fallback

GitHub Actions 가 막혔거나 빠르게 hot-fix 가 필요하면 `scripts/release.sh`
가 동일한 산출물을 로컬에서 만들어준다. 그 후 `gh release upload --clobber`
로 직접 publish.
