use crate::error::DanbiResult;
use crate::vault;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VaultTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub default_domains: Vec<String>,
    /// Sub-folders auto-created inside every new project using this template
    /// (e.g. "daily", "notes"). Kept flat — single level.
    #[serde(default)]
    pub default_folders: Vec<String>,
    pub sample_project: Option<SampleProject>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SampleProject {
    pub name: String,
    pub files: Vec<SampleFile>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SampleFile {
    pub name: String,
    pub content: String,
}

pub fn list_templates() -> Vec<VaultTemplate> {
    vec![
        VaultTemplate {
            id: "developer".into(),
            name: "개발 프로젝트".into(),
            description: "사이드 프로젝트용 빈 프로젝트. daily / notes / decisions 폴더가 준비됩니다. 파일은 필요할 때 직접 만드세요.".into(),
            icon: "code".into(),
            default_domains: vec![],
            default_folders: vec!["daily".into(), "notes".into(), "decisions".into()],
            sample_project: None,
        },
        VaultTemplate {
            id: "knowledge".into(),
            name: "개인 지식".into(),
            description: "카파시식 LLM Wiki. 생각·독서·사람·아이디어를 한 곳에 쌓아둡니다.".into(),
            icon: "brain".into(),
            default_domains: vec![
                "notes.md".into(),
                "ideas.md".into(),
                "reading.md".into(),
                "people.md".into(),
            ],
            default_folders: vec!["daily".into()],
            sample_project: Some(SampleProject {
                name: "나의노트".into(),
                files: vec![
                    SampleFile {
                        name: "notes.md".into(),
                        content: "# 노트\n\n매일의 생각·메모·관찰을 쌓아두는 곳.\n\n## 오늘\n\n- 단비를 처음 사용한 날\n".into(),
                    },
                    SampleFile {
                        name: "ideas.md".into(),
                        content: "# 아이디어\n\n떠오른 프로덕트·글·실험 아이디어.\n".into(),
                    },
                    SampleFile {
                        name: "reading.md".into(),
                        content: "# 독서\n\n읽은 책·논문·글에서 건진 발췌.\n".into(),
                    },
                    SampleFile {
                        name: "people.md".into(),
                        content: "# 사람\n\n만난 사람과 대화에서 배운 것.\n".into(),
                    },
                ],
            }),
        },
        VaultTemplate {
            id: "reading".into(),
            name: "독서 일지".into(),
            description: "책/논문 한 권당 프로젝트 하나. 요약·인용·질문을 분리 기록합니다.".into(),
            icon: "book".into(),
            default_domains: vec![
                "summary.md".into(),
                "quotes.md".into(),
                "questions.md".into(),
            ],
            default_folders: vec![],
            sample_project: Some(SampleProject {
                name: "예시책".into(),
                files: vec![
                    SampleFile {
                        name: "summary.md".into(),
                        content: "# 요약\n\n이 책의 핵심 주장을 한 문단으로.\n".into(),
                    },
                    SampleFile {
                        name: "quotes.md".into(),
                        content: "# 인용\n\n- \"기억할 가치가 있는 문장\" (p. 42)\n".into(),
                    },
                    SampleFile {
                        name: "questions.md".into(),
                        content: "# 질문\n\n읽으면서 떠오른 의문.\n".into(),
                    },
                ],
            }),
        },
        VaultTemplate {
            id: "empty".into(),
            name: "비어있음".into(),
            description: "아무것도 만들지 않습니다. 직접 프로젝트·도메인을 구성하고 싶을 때.".into(),
            icon: "circle".into(),
            default_domains: vec![],
            default_folders: vec![],
            sample_project: None,
        },
    ]
}

pub fn get_template(id: &str) -> Option<VaultTemplate> {
    list_templates().into_iter().find(|t| t.id == id)
}

/// Applies a template to an existing vault: creates the sample project (if any)
/// with the template's files + declared sub-folders. `default_domains`/`default_folders`
/// are also persisted to config.json by the caller so subsequent projects use them.
pub fn apply_template(vault: &Path, template: &VaultTemplate) -> DanbiResult<()> {
    vault::init_vault(vault)?;
    let Some(sample) = &template.sample_project else {
        return Ok(());
    };
    let _ = vault::create_project_with_folders(
        vault,
        &sample.name,
        &template.default_domains,
        &template.default_folders,
    );
    for f in &sample.files {
        let _ = vault::write_doc(vault, &sample.name, &f.name, &f.content);
    }
    Ok(())
}
