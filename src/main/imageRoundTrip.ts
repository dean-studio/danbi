/**
 * BlockNote ↔ Markdown 이미지 왕복 유틸.
 *
 * 문제: BlockNote 0.50은 `previewWidth`가 설정된 이미지 블록을
 * `blocksToMarkdownLossy` 결과에서 누락시키고, 반대로 마크다운에는 크기
 * 정보를 담을 자리가 없다. 우리는 두 방향 모두를 직접 제어해서 리사이즈 된
 * 크기가 라운드트립을 살아남게 한다.
 *
 * 저장 방향:
 *  1. editor.document 를 재귀 복사하면서 이미지 블록을 "paragraph with inline
 *     <img src width alt>" 블록으로 대체한다.
 *  2. 그 복사본을 blocksToMarkdownLossy 에 넣으면 `<img>` HTML이 md 본문에
 *     그대로 직렬화된다 (BlockNote paragraph text는 escape 없이 뽑힘).
 *
 * 로드 방향:
 *  1. md 텍스트에서 `<img …>` 구문을 고유 placeholder 토큰으로 치환한다.
 *  2. BlockNote 가 markdown 을 블록으로 파싱한 뒤, 우리가 블록 트리를 걸어
 *     placeholder 가 박힌 paragraph 를 찾아 이미지 블록으로 복원한다.
 *
 * 순서에 의존하지 않고 각 이미지에 고유 id를 부여하기 때문에 다른 HTML/markdown
 * 변환 경로에서도 어긋나지 않는다.
 */

import type {
  Block,
  BlockNoteEditor,
  PartialBlock,
} from "@blocknote/core";

type AnyBlock = {
  id?: string;
  type?: string;
  props?: Record<string, unknown>;
  content?: unknown;
  children?: AnyBlock[];
};

const PLACEHOLDER_PREFIX = "@@DANBI_IMG_";
const PLACEHOLDER_SUFFIX = "@@";

type ImgMeta = {
  url: string;
  width?: number;
  alt?: string;
};

function escapeAttr(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function unescapeAttr(s: string): string {
  return s
    .replace(/&quot;/g, '"')
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

/**
 * Walks the document and swaps every image block for a paragraph carrying an
 * inline `<img>` tag. Returns the transformed copy (leaves the live document
 * alone).
 */
export function imageBlocksToHtmlParagraphs(
  doc: Block[] | AnyBlock[],
): PartialBlock[] {
  const visit = (list: AnyBlock[] | undefined): PartialBlock[] => {
    if (!list) return [];
    return list.map((b): PartialBlock => {
      if (b.type === "image" && b.props && typeof b.props.url === "string") {
        const url = b.props.url as string;
        const width = b.props.previewWidth ?? b.props.width;
        const alt =
          (b.props.caption as string | undefined) ??
          (b.props.name as string | undefined) ??
          "";
        const widthAttr =
          typeof width === "number" && isFinite(width)
            ? ` width="${Math.round(width)}"`
            : "";
        const inline = `<img src="${escapeAttr(url)}"${widthAttr} alt="${escapeAttr(
          alt,
        )}">`;
        const para: PartialBlock = {
          type: "paragraph",
          content: inline,
        };
        if (b.children && b.children.length) {
          (para as PartialBlock & { children: PartialBlock[] }).children = visit(
            b.children,
          );
        }
        return para;
      }
      // Non-image: pass through, recurse into children if present.
      const copy: AnyBlock = { ...b };
      if (b.children && b.children.length) {
        (copy as AnyBlock).children = visit(b.children) as unknown as AnyBlock[];
      }
      return copy as PartialBlock;
    });
  };
  return visit(doc as AnyBlock[]);
}

/**
 * Replaces every `<img …>` occurrence in markdown with a unique placeholder
 * token and returns the rewritten markdown plus a map to restore images later.
 */
export function stashImagesForParse(md: string): {
  stripped: string;
  imgs: Map<string, ImgMeta>;
} {
  const imgs = new Map<string, ImgMeta>();
  let n = 0;
  const re = /<img\b[^>]*?>/gi;
  const stripped = md.replace(re, (match) => {
    const token = `${PLACEHOLDER_PREFIX}${n}${PLACEHOLDER_SUFFIX}`;
    n += 1;
    imgs.set(token, parseImgTag(match));
    return token;
  });
  return { stripped, imgs };
}

function parseImgTag(tag: string): ImgMeta {
  const src = tag.match(/\bsrc\s*=\s*["']([^"']+)["']/i);
  const width = tag.match(/\bwidth\s*=\s*["']?(\d+)/i);
  const alt = tag.match(/\balt\s*=\s*["']([^"']*)["']/i);
  return {
    url: src ? unescapeAttr(src[1]) : "",
    width: width ? parseInt(width[1], 10) : undefined,
    alt: alt ? unescapeAttr(alt[1]) : undefined,
  };
}

/**
 * Walks a parsed block tree and swaps any paragraph whose text content is a
 * single placeholder token (or exactly one token with no surrounding text) for
 * a real image block. Preserves the previously captured width.
 */
export function restoreImageBlocks(
  blocks: PartialBlock[],
  imgs: Map<string, ImgMeta>,
): PartialBlock[] {
  if (imgs.size === 0) return blocks;

  const textOf = (b: AnyBlock): string => {
    // BlockNote 0.50 paragraph content is usually string or InlineContent[].
    const c = b.content;
    if (!c) return "";
    if (typeof c === "string") return c;
    if (Array.isArray(c)) {
      return (c as Array<{ type?: string; text?: string }>)
        .map((x) => (typeof x?.text === "string" ? x.text : ""))
        .join("");
    }
    return "";
  };

  const visit = (list: PartialBlock[]): PartialBlock[] => {
    const out: PartialBlock[] = [];
    for (const b of list) {
      const anyB = b as AnyBlock;
      if (anyB.type === "paragraph") {
        const plain = textOf(anyB).trim();
        // If this paragraph is exactly a placeholder, replace it wholesale.
        const exact = imgs.get(plain);
        if (exact) {
          out.push(buildImageBlock(exact));
          continue;
        }
        // If it contains placeholders mixed with text, split them into multiple blocks.
        if (containsPlaceholder(plain, imgs)) {
          const parts = splitAroundPlaceholders(plain, imgs);
          for (const p of parts) {
            if (p.kind === "img") {
              out.push(buildImageBlock(p.meta));
            } else if (p.text.length > 0) {
              out.push({ type: "paragraph", content: p.text } as PartialBlock);
            }
          }
          continue;
        }
      }
      // Recurse into children.
      if (anyB.children && Array.isArray(anyB.children)) {
        const nextChildren = visit(
          anyB.children as unknown as PartialBlock[],
        );
        out.push({
          ...(b as object),
          children: nextChildren,
        } as PartialBlock);
      } else {
        out.push(b);
      }
    }
    return out;
  };

  return visit(blocks);
}

function buildImageBlock(meta: ImgMeta): PartialBlock {
  const props: Record<string, unknown> = {
    url: meta.url,
  };
  if (typeof meta.width === "number") props.previewWidth = meta.width;
  if (meta.alt) props.caption = meta.alt;
  return {
    type: "image",
    props,
  } as PartialBlock;
}

function containsPlaceholder(
  text: string,
  imgs: Map<string, ImgMeta>,
): boolean {
  for (const token of imgs.keys()) {
    if (text.includes(token)) return true;
  }
  return false;
}

type Part =
  | { kind: "text"; text: string }
  | { kind: "img"; meta: ImgMeta };

function splitAroundPlaceholders(
  text: string,
  imgs: Map<string, ImgMeta>,
): Part[] {
  // Greedy tokenize: find earliest placeholder, split, recurse.
  const out: Part[] = [];
  let remaining = text;
  // Precompile a single regex covering all tokens.
  const tokenList = Array.from(imgs.keys());
  if (tokenList.length === 0) {
    return [{ kind: "text", text }];
  }
  const tokenRe = new RegExp(
    tokenList.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|"),
    "g",
  );
  let lastIndex = 0;
  for (const m of remaining.matchAll(tokenRe)) {
    const idx = m.index ?? 0;
    if (idx > lastIndex) {
      out.push({
        kind: "text",
        text: remaining.slice(lastIndex, idx).trim(),
      });
    }
    const meta = imgs.get(m[0]);
    if (meta) out.push({ kind: "img", meta });
    lastIndex = idx + m[0].length;
  }
  if (lastIndex < remaining.length) {
    out.push({ kind: "text", text: remaining.slice(lastIndex).trim() });
  }
  return out.filter((p) => (p.kind === "text" ? p.text.length > 0 : true));
}

// ---------- Convenience wrappers for DocView ----------

import { rewriteMdToWikiLinks, rewriteWikiLinksToMd } from "@/main/wikiLinks";

export function exportBlocksToMarkdown(
  editor: BlockNoteEditor,
): string {
  const transformed = imageBlocksToHtmlParagraphs(
    editor.document as unknown as AnyBlock[],
  );
  // blocksToMarkdownLossy accepts PartialBlock[]; our inline HTML survives as
  // raw text inside the paragraph block.
  const md = editor.blocksToMarkdownLossy(transformed);
  // BlockNote 는 anchor 를 표준 markdown link 로 export 한다. `danbi:` 프로토콜
  // 링크는 사용자가 디스크에서 다시 보면 어색하니까 wiki 문법으로 되돌린다.
  return rewriteMdToWikiLinks(md);
}

export function parseMarkdownRestoringImages(
  editor: BlockNoteEditor,
  md: string,
): PartialBlock[] {
  // 디스크의 [[wiki-link]] 를 BlockNote 가 일반 link 로 인식하도록 markdown
  // anchor 로 사전 변환한다. export 단계에서 다시 [[...]] 로 되돌아가므로
  // 디스크 파일 형식은 손상되지 않는다.
  const wikified = rewriteWikiLinksToMd(md);
  const { stripped, imgs } = stashImagesForParse(wikified);
  const parsed = editor.tryParseMarkdownToBlocks(stripped);
  return restoreImageBlocks(parsed as unknown as PartialBlock[], imgs);
}
