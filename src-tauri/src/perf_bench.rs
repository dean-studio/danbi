//! 검색·그래프 핫패스 실측 벤치 (`cargo test -- --ignored perf_bench`).
//!
//! 실제 vault(`DANBI_BENCH_VAULT`, 기본 `~/Danbi_Vault`)의 `Projects/` 와
//! `.danbi/vectors.json` 을 tempdir 로 복사한 뒤 그 복사본만 읽고 쓴다 —
//! 원본 vault 에는 아무것도 쓰지 않는다.

use std::path::{Path, PathBuf};
use std::time::Instant;

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for de in std::fs::read_dir(src).unwrap() {
        let de = de.unwrap();
        let p = de.path();
        let name = de.file_name();
        if name == ".git" {
            continue;
        }
        let target = dst.join(&name);
        if p.is_dir() {
            copy_dir(&p, &target);
        } else {
            std::fs::copy(&p, &target).unwrap();
        }
    }
}

fn bench_vault() -> PathBuf {
    let src = std::env::var("DANBI_BENCH_VAULT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap().join("Danbi_Vault"));
    assert!(src.join("Projects").exists(), "no vault at {}", src.display());
    let dst = std::env::temp_dir().join(format!("danbi-perf-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    copy_dir(&src.join("Projects"), &dst.join("Projects"));
    std::fs::create_dir_all(dst.join(".danbi")).unwrap();
    if src.join(".danbi/vectors.json").exists() {
        std::fs::copy(src.join(".danbi/vectors.json"), dst.join(".danbi/vectors.json")).unwrap();
    }
    dst
}

fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    eprintln!("[bench] {label:<44} {:>8.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    out
}

#[test]
#[ignore]
fn perf_bench_hot_paths() {
    let vault = bench_vault();
    let md_count = walkdir_count(&vault.join("Projects"));
    eprintln!("[bench] vault copy: {} ({md_count} .md)", vault.display());

    // ---- vectors.json ----
    for i in 0..3 {
        timed(&format!("vector::load (disk parse) #{i}"), || {
            crate::vector::load(&vault).unwrap().entries.len()
        });
    }
    for i in 0..3 {
        timed(&format!("vector::load_cached #{i}"), || {
            crate::vector::load_cached(&vault).unwrap().entries.len()
        });
    }
    let idx = crate::vector::load_cached(&vault).unwrap();
    let dim = idx.entries.values().map(|e| e.embedding.len()).max().unwrap_or(0);
    let q: Vec<f32> = (0..dim).map(|i| ((i * 7919) % 101) as f32 / 101.0 - 0.5).collect();
    timed("vector::query (cosine scan)", || crate::vector::query(&idx, &q, 24).len());

    // ---- fingerprint / snapshot ----
    for i in 0..3 {
        timed(&format!("search::fingerprint_vault #{i}"), || {
            crate::search::fingerprint_vault(&vault).unwrap()
        });
    }

    // ---- tantivy corpus ----
    timed("full_search cold (first build)", || {
        crate::search::full_search(&vault, "리프레시 토큰", 24).unwrap().len()
    });
    for i in 0..3 {
        timed(&format!("full_search warm #{i}"), || {
            crate::search::full_search(&vault, "MCP 서버 연결", 24).unwrap().len()
        });
    }
    // 파일 하나만 바꿨을 때 재색인 비용.
    let touched = first_md(&vault.join("Projects")).unwrap();
    let mut body = std::fs::read_to_string(&touched).unwrap();
    body.push_str("\n\nbench-touch 고유토큰zq\n");
    std::fs::write(&touched, &body).unwrap();
    timed("full_search after 1-file change", || {
        crate::search::full_search(&vault, "고유토큰zq", 24).unwrap().len()
    });
    timed("full_search warm after change", || {
        crate::search::full_search(&vault, "일정", 24).unwrap().len()
    });
    // 삭제도 반영되는지 + 비용.
    std::fs::remove_file(&touched).unwrap();
    let n = timed("full_search after 1-file delete", || {
        crate::search::full_search(&vault, "고유토큰zq", 24).unwrap().len()
    });
    assert_eq!(n, 0, "deleted doc must leave the index");

    // ---- links / graph ----
    for i in 0..3 {
        timed(&format!("links::build_index #{i}"), || {
            crate::links::build_index(&vault).unwrap().outgoing.len()
        });
    }
    for i in 0..3 {
        timed(&format!("links::build_index_cached #{i}"), || {
            crate::links::build_index_cached(&vault).unwrap().outgoing.len()
        });
    }
    for i in 0..3 {
        timed(&format!("graph::existing_doc_ids #{i}"), || {
            crate::graph::existing_doc_ids(&vault).unwrap().len()
        });
    }
    for i in 0..3 {
        timed(&format!("graph::existing_doc_ids_cached #{i}"), || {
            crate::graph::existing_doc_ids_cached(&vault).unwrap().len()
        });
    }
    let project = "단비";
    for i in 0..2 {
        timed(&format!("graph::build_graph({project}) #{i}"), || {
            crate::graph::build_graph(&vault, Some(project)).unwrap().nodes.len()
        });
    }
    for i in 0..3 {
        timed(&format!("graph::build_graph_cached({project}) #{i}"), || {
            crate::graph::build_graph_cached(&vault, Some(project)).unwrap().nodes.len()
        });
    }
    timed("graph::build_graph(all)", || {
        crate::graph::build_graph(&vault, None).unwrap().nodes.len()
    });
    let doc = crate::graph::build_graph_cached(&vault, Some(project))
        .unwrap()
        .nodes
        .first()
        .map(|n| n.domain.clone())
        .unwrap();
    for i in 0..3 {
        timed(&format!("graph::related({project}/{doc}) #{i}"), || {
            crate::graph::related(&vault, project, &doc, 10).unwrap().related.len()
        });
    }

    let _ = std::fs::remove_dir_all(&vault);
}

fn walkdir_count(dir: &Path) -> usize {
    let mut n = 0;
    for de in std::fs::read_dir(dir).unwrap() {
        let p = de.unwrap().path();
        if p.is_dir() {
            n += walkdir_count(&p);
        } else if p.extension().map(|e| e == "md").unwrap_or(false) {
            n += 1;
        }
    }
    n
}

fn first_md(dir: &Path) -> Option<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if let Some(f) = first_md(&p) {
                return Some(f);
            }
        } else if p.extension().map(|e| e == "md").unwrap_or(false) {
            return Some(p);
        }
    }
    None
}
