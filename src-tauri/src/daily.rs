use crate::error::DanbiResult;
use crate::vault::PROJECTS_DIRNAME;
use chrono::{Datelike, Local, NaiveDate};
use serde::Serialize;
use std::path::Path;

pub const DAILY_DIRNAME: &str = "daily";

#[derive(Debug, Serialize, Clone)]
pub struct DailyNoteRef {
    pub project: String,
    /// Relative to the project root, e.g. "daily/2026-05-11.md".
    pub domain: String,
    pub date: String, // "YYYY-MM-DD"
    pub bytes: u64,
    /// Last modification time of the note, ms since epoch. Used in the
    /// home dashboard so the user can see when each daily note was last
    /// touched. None when filesystem doesn't expose mtime.
    pub modified_ms: Option<i64>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DailySnapshot {
    /// Today's date — "YYYY-MM-DD" in the user's local timezone.
    pub today: String,
    /// Daily notes that exist for today, one per project.
    pub today_notes: Vec<DailyNoteRef>,
    /// Reminiscence — daily notes from exactly N days ago still present.
    pub one_week_ago: Vec<DailyNoteRef>,
    pub one_month_ago: Vec<DailyNoteRef>,
    pub one_year_ago: Vec<DailyNoteRef>,
}

fn today_str() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn nth_days_ago(n: i64) -> String {
    let now = Local::now().date_naive();
    let d = now - chrono::Duration::days(n);
    d.format("%Y-%m-%d").to_string()
}

/// Returns the NaiveDate `n` days ago — used when `n` is actually "1 month"
/// or "1 year" so we can use Datelike math rather than day arithmetic.
fn date_shift(years: i32, months: i32, days: i64) -> NaiveDate {
    let now = Local::now().date_naive();
    let mut y = now.year();
    let mut m = now.month() as i32;
    let d = now.day();

    y -= years;
    m -= months;
    while m <= 0 {
        m += 12;
        y -= 1;
    }
    while m > 12 {
        m -= 12;
        y += 1;
    }

    // Use the 1st if the target day doesn't exist (e.g. Feb 30).
    let shifted = NaiveDate::from_ymd_opt(y, m as u32, d)
        .or_else(|| NaiveDate::from_ymd_opt(y, m as u32, 1))
        .unwrap_or(now);

    shifted - chrono::Duration::days(days)
}

fn iter_daily_files(vault: &Path) -> DanbiResult<Vec<DailyNoteRef>> {
    let projects_root = vault.join(PROJECTS_DIRNAME);
    if !projects_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&projects_root)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(project) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if project.starts_with('.') {
            continue;
        }
        let daily_dir = p.join(DAILY_DIRNAME);
        if !daily_dir.exists() {
            continue;
        }
        for de in std::fs::read_dir(&daily_dir)? {
            let de = de?;
            let path = de.path();
            if !path.is_file() {
                continue;
            }
            let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !fname.ends_with(".md") || fname.starts_with('.') {
                continue;
            }
            let date = fname.trim_end_matches(".md").to_string();
            if NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
                continue;
            }
            let meta = std::fs::metadata(&path).ok();
            let bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified_ms = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            out.push(DailyNoteRef {
                project: project.to_string(),
                domain: format!("{DAILY_DIRNAME}/{fname}"),
                date,
                bytes,
                modified_ms,
            });
        }
    }
    Ok(out)
}

fn filter_on_date<'a>(all: &'a [DailyNoteRef], date: &str) -> Vec<DailyNoteRef> {
    all.iter().filter(|n| n.date == date).cloned().collect()
}

pub fn snapshot(vault: &Path) -> DanbiResult<DailySnapshot> {
    let all = iter_daily_files(vault)?;
    let today = today_str();
    let week = nth_days_ago(7);
    let month = date_shift(0, 1, 0).format("%Y-%m-%d").to_string();
    let year = date_shift(1, 0, 0).format("%Y-%m-%d").to_string();

    Ok(DailySnapshot {
        today,
        today_notes: filter_on_date(&all, &today_str()),
        one_week_ago: filter_on_date(&all, &week),
        one_month_ago: filter_on_date(&all, &month),
        one_year_ago: filter_on_date(&all, &year),
    })
}

/// Creates (or returns) today's daily note for a project. Idempotent.
pub fn ensure_today_note(vault: &Path, project: &str) -> DanbiResult<String> {
    let date = today_str();
    let projects_root = vault.join(PROJECTS_DIRNAME);
    let daily_dir = projects_root.join(project).join(DAILY_DIRNAME);
    std::fs::create_dir_all(&daily_dir)?;
    let filename = format!("{date}.md");
    let path = daily_dir.join(&filename);
    if !path.exists() {
        let header = format!(
            "# {date}\n\n<!-- daily note — 단비가 오늘의 작업을 여기에 쌓아둡니다 -->\n",
        );
        std::fs::write(&path, header)?;
    }
    Ok(format!("{DAILY_DIRNAME}/{filename}"))
}
