/**
 * Wiki-link round-trip between markdown source and BlockNote.
 *
 * 디스크 위 마크다운은 단비의 표준 wiki-link 문법으로 저장된다:
 *   [[domain.md]]                같은 프로젝트
 *   [[Project/domain.md]]        다른 프로젝트
 *   [[domain.md|alias text]]     파이프 별칭
 *
 * BlockNote 는 비표준 URL scheme (예: `danbi:`) 을 markdown link 로 인식하지
 * 않아서 anchor 로 렌더링되지 않는다. 그래서 일단 표준 https:// URL 로 위장
 * 해서 BlockNote 가 정상적으로 anchor 를 만들도록 하고, export 시 다시
 * wiki 문법으로 되돌린다. 호스트는 절대로 외부에 닿지 않을 더미값
 * (`danbi.invalid`) 을 쓴다 — RFC 6761 가 보장하는 미해석 도메인이다.
 *
 *   load:   [[notes/next-up.md|이름]]  →  [이름](https://danbi.invalid/notes/next-up.md)
 *   load:   [[Bonny/ui.md]]            →  [Bonny/ui.md](https://danbi.invalid/Bonny/ui.md)
 *   save:   [text](https://danbi.invalid/x)   →  [[x|text]]
 *
 * DocView 의 클릭 핸들러가 호스트가 `danbi.invalid` 인 anchor 를 가로채서
 * 외부 브라우저로 나가지 않게 막고 store 의 selectDomain 으로 라우팅한다.
 */

const WIKI_LINK_RE = /\[\[([^\]\n|]+?)(?:\|([^\]\n]+?))?\]\]/g;
const WIKI_HOST = "danbi.invalid";
const WIKI_HREF_PREFIX = `https://${WIKI_HOST}/`;

/** Encode a wiki target so it survives the markdown link href. The target
 *  contains "/" (path) and "." (extension) which are URL-safe, but spaces
 *  / non-ASCII inside aliases or paths need percent encoding. */
function encodeTarget(target: string): string {
  return target
    .split("/")
    .map((seg) => encodeURIComponent(seg))
    .join("/");
}

function decodeTarget(target: string): string {
  try {
    return target
      .split("/")
      .map((seg) => decodeURIComponent(seg))
      .join("/");
  } catch {
    return target;
  }
}

/** md (disk) → md (with stub https:// links) — applied right before
 *  BlockNote parses the markdown so the resulting blocks include real
 *  link inline content. */
export function rewriteWikiLinksToMd(md: string): string {
  return md.replace(WIKI_LINK_RE, (_, target: string, alias?: string) => {
    const t = target.trim();
    const text = (alias?.trim() || t);
    return `[${text}](${WIKI_HREF_PREFIX}${encodeTarget(t)})`;
  });
}

/** md (BlockNote export) → md (disk) — collapse stub hrefs back to wiki
 *  syntax. When the visible text equals the target we drop the alias so
 *  the file stays visually clean. */
export function rewriteMdToWikiLinks(md: string): string {
  const escapedHost = WIKI_HOST.replace(/\./g, "\\.");
  const re = new RegExp(
    `\\[([^\\]\\n]+?)\\]\\(https?://${escapedHost}/([^)\\n]+?)\\)`,
    "g",
  );
  return md.replace(re, (_, text: string, target: string) => {
    const t = decodeTarget(target).trim();
    const alias = text.trim();
    if (!alias || alias === t) {
      return `[[${t}]]`;
    }
    return `[[${t}|${alias}]]`;
  });
}

/** Quick check used by the DocView click handler — true when an anchor's
 *  href points at our stub host rather than a real external URL. */
export function isWikiHref(href: string | null | undefined): boolean {
  if (!href) return false;
  return href.startsWith(WIKI_HREF_PREFIX) ||
    href.startsWith(`http://${WIKI_HOST}/`);
}

/** Pull the wiki target back out of a stub https://danbi.invalid/... href. */
export function parseWikiHref(href: string): { project: string | null; domain: string } | null {
  if (!isWikiHref(href)) return null;
  const idx = href.indexOf(WIKI_HOST);
  if (idx < 0) return null;
  const after = href.slice(idx + WIKI_HOST.length);
  const path = after.startsWith("/") ? after.slice(1) : after;
  const target = decodeTarget(path).trim();
  if (target.length === 0) return null;
  // `Project/domain.md` vs same-project `domain.md`. We treat the FIRST
  // "/" as the project separator only if there's at least one path
  // segment after it that looks like a filename.
  const slash = target.indexOf("/");
  if (slash > 0) {
    const head = target.slice(0, slash);
    const rest = target.slice(slash + 1);
    // 너무 긴 head (예: notes/foo/bar.md 의 "notes") 는 sub-folder 일 수도
    // 있는데, 단비의 wiki 문법상 "프로젝트/...md" 는 head 가 최상위
    // 프로젝트명이라는 가정에 의존한다. 호출자(DocView) 가 실제 vault
    // 트리에서 head 가 프로젝트인지 확인한 뒤 라우팅하므로 여기선 그대로
    // 분리만 한다.
    return { project: head, domain: rest };
  }
  return { project: null, domain: target };
}
