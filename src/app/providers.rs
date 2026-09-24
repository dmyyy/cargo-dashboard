use super::App;
use cargo_metadata::MetadataCommand;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Debug, Clone, Default)]
pub struct ProjectMetadataSummary {
    pub package_name: String,
    pub package_version: String,
    pub description: String,
    pub git_branch: String,
    pub git_status: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CiRun {
    pub status: String,
    pub branch: String,
    pub title: String,
    pub created_at: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiRunsData {
    pub repo: String,
    pub runs: Vec<CiRun>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageStat {
    pub name: String,
    pub code: u64,
    pub blanks: u64,
    pub comments: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguagesData {
    pub languages: Vec<LanguageStat>,
}
pub(super) fn normalize_github_remote(remote: &str) -> Option<String> {
    if let Some(rest) = remote.strip_prefix("git@github.com:") {
        return Some(format!(
            "https://github.com/{}",
            rest.trim_end_matches(".git")
        ));
    }
    if let Some(rest) = remote.strip_prefix("https://github.com/") {
        return Some(format!(
            "https://github.com/{}",
            rest.trim_end_matches(".git")
        ));
    }
    None
}

pub(super) fn shell_url_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

impl App {
    pub fn project_metadata(&self) -> Option<&ProjectMetadataSummary> {
        let project_path = self.current_project()?.path.clone();
        self.metadata_cache.get(&project_path)
    }

    pub fn project_ci_runs(&self) -> Option<&CiRunsData> {
        let project_path = self.current_project()?.path.clone();
        self.ci_runs_cache
            .get(&project_path)?
            .as_ref()
            .filter(|ci| !ci.runs.is_empty())
    }

    pub fn visible_ci_runs(&self) -> impl Iterator<Item = &CiRun> {
        self.filtered_ci_runs
            .iter()
            .filter_map(|&index| self.project_ci_runs()?.runs.get(index))
    }

    pub fn current_ci_run(&self) -> Option<&CiRun> {
        let visible_index = usize::try_from(self.ci_cursor).ok()?;
        let ci_index = *self.filtered_ci_runs.get(visible_index)?;
        self.project_ci_runs()?.runs.get(ci_index)
    }

    pub fn project_languages(&self) -> Option<&LanguagesData> {
        let project_path = self.current_project()?.path.clone();
        self.languages_cache.get(&project_path)?.as_ref()
    }
    pub fn open_selected_ci_run(&mut self) -> color_eyre::Result<()> {
        let Some(run) = self.current_ci_run() else {
            return Ok(());
        };

        Command::new("open")
            .arg(&run.url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(())
    }
}
pub(super) fn discover_project_metadata(project_path: &Path) -> ProjectMetadataSummary {
    let metadata = MetadataCommand::new()
        .current_dir(project_path)
        .no_deps()
        .exec()
        .ok();

    let workspace_root = metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path() == project_path)
        .unwrap_or(false);
    let package = metadata
        .as_ref()
        .and_then(|metadata| metadata.root_package());

    let package_name = if workspace_root && package.is_none() {
        project_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "—".to_string())
    } else {
        package
            .map(|pkg| pkg.name.to_string())
            .or_else(|| {
                project_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "—".to_string())
    };
    let package_version = package
        .map(|pkg| pkg.version.to_string())
        .unwrap_or_else(|| "—".to_string());
    let description = package
        .and_then(|pkg| pkg.description.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "No description.".to_string());
    let git_branch = current_git_branch(project_path).unwrap_or_else(|| "—".to_string());
    let git_status = git_status(project_path);

    ProjectMetadataSummary {
        package_name,
        package_version,
        description,
        git_branch,
        git_status,
    }
}

pub(super) fn current_git_branch(project_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(project_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8(output.stdout).ok()?;
    let branch = branch.trim();
    if branch.is_empty() {
        None
    } else {
        Some(branch.to_string())
    }
}

pub(super) fn git_status(project_path: &Path) -> String {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(project_path)
        .output();

    let Ok(output) = output else {
        return "UNTRACKED".to_string();
    };

    if !output.status.success() {
        return "UNTRACKED".to_string();
    }

    if String::from_utf8_lossy(&output.stdout).trim().is_empty() {
        "CLEAN".to_string()
    } else {
        "DIRTY".to_string()
    }
}

pub(super) fn discover_ci_runs(project_path: &Path) -> Option<CiRunsData> {
    let repo_url = github_repo_url(project_path)?;
    let owner_repo = repo_url.strip_prefix("https://github.com/")?;
    let current_branch = current_git_branch(project_path);
    let runs_path = if let Some(branch) = current_branch.as_deref() {
        format!(
            "repos/{owner_repo}/actions/runs?per_page=25&branch={}",
            shell_url_encode(branch)
        )
    } else {
        format!("repos/{owner_repo}/actions/runs?per_page=25")
    };
    let output = Command::new("gh")
        .args(["api", &runs_path])
        .current_dir(project_path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    #[derive(Deserialize)]
    struct GhRunsResponse {
        workflow_runs: Vec<GhRun>,
    }

    #[derive(Deserialize)]
    struct GhRun {
        html_url: String,
        display_title: Option<String>,
        head_branch: Option<String>,
        created_at: String,
        conclusion: Option<String>,
        status: String,
    }

    let response: GhRunsResponse = serde_json::from_slice(&output.stdout).ok()?;
    Some(CiRunsData {
        repo: owner_repo.to_string(),
        runs: response
            .workflow_runs
            .into_iter()
            .map(|run| CiRun {
                status: run.conclusion.unwrap_or(run.status),
                branch: run.head_branch.unwrap_or_else(|| "—".to_string()),
                title: run.display_title.unwrap_or_else(|| "—".to_string()),
                created_at: run.created_at,
                url: run.html_url,
            })
            .collect(),
    })
}

pub(super) fn github_repo_url(project_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(project_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let remote = String::from_utf8(output.stdout).ok()?;
    normalize_github_remote(remote.trim())
}

pub(super) fn discover_languages(project_path: &Path) -> Option<LanguagesData> {
    let output = Command::new("tokei")
        .args([".", "--output", "json"])
        .current_dir(project_path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let object = value.as_object()?;
    let mut languages = object
        .iter()
        .filter(|(name, _)| name.as_str() != "Total")
        .filter_map(|(name, stats)| {
            let stats = stats.as_object()?;
            Some(LanguageStat {
                name: name.clone(),
                blanks: stats.get("blanks")?.as_u64()?,
                code: stats.get("code")?.as_u64()?,
                comments: stats.get("comments")?.as_u64()?,
            })
        })
        .filter(|stat| stat.code > 0)
        .collect::<Vec<_>>();

    languages.sort_by(|a, b| b.code.cmp(&a.code).then_with(|| a.name.cmp(&b.name)));
    if languages.is_empty() {
        None
    } else {
        Some(LanguagesData { languages })
    }
}
