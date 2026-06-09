use crate::{
    event::{AppEvent, Event, EventHandler},
    ui,
};
use cargo_metadata::{MetadataCommand, Target as CargoTarget};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo::{
    Config, Nucleo, Utf32String,
    pattern::{CaseMatching, Normalization},
};
use ratatui::{
    DefaultTerminal,
    style::{Color, Style},
};
use ratatui_cheese::spinner::{Spinner, SpinnerState, SpinnerType};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Instant,
};
use tokio::sync::{Semaphore, mpsc};
use tui_input::{Input, InputRequest};

#[derive(Debug, Clone)]
pub struct Project {
    pub name: String,
    pub path: PathBuf,
    pub has_git_dir: bool,
    pub is_workspace: bool,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub kind: String,
    pub name: String,
    pub path: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Projects,
    CiRuns,
    Targets,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetStatusKind {
    Building,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunProfile {
    Debug,
    Release,
}

#[derive(Debug, Clone, Default)]
pub struct ProcessStats {
    pub cpu_percent: Option<f32>,
    pub memory_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TargetStatus {
    pub kind: TargetStatusKind,
    pub project_name: String,
    pub project_path: PathBuf,
    pub target_name: String,
    pub target_kind: String,
    pub profile: RunProfile,
    pub status_path: PathBuf,
    pub log_path: PathBuf,
    pub pid_path: PathBuf,
    pub pid: Option<u32>,
    pub started_at: Option<Instant>,
    pub stats: ProcessStats,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RecentProjects {
    pub entries: HashMap<String, u64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BookmarkedProjects {
    pub entries: HashSet<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectMetadataSummary {
    pub package_name: String,
    pub package_version: String,
    pub description: String,
    pub git_branch: String,
    pub git_status: String,
}

#[derive(Debug)]
pub enum BackgroundResult {
    Metadata(PathBuf, ProjectMetadataSummary),
    GitStatus(PathBuf, String),
    Size(PathBuf, u64),
    CiRuns(PathBuf, Option<CiRunsData>),
    Languages(PathBuf, Option<LanguagesData>),
    Targets(PathBuf, Vec<Target>),
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

pub struct App {
    pub running: bool,
    pub counter: u8,
    pub cursor: isize,
    pub target_cursor: isize,
    pub ci_cursor: isize,
    pub targets: Vec<Target>,
    pub target_cache: HashMap<PathBuf, Vec<Target>>,
    pub metadata_cache: HashMap<PathBuf, ProjectMetadataSummary>,
    pub ci_runs_cache: HashMap<PathBuf, Option<CiRunsData>>,
    pub languages_cache: HashMap<PathBuf, Option<LanguagesData>>,
    pub git_status_cache: HashMap<PathBuf, String>,
    pub size_cache: HashMap<PathBuf, u64>,
    pub loading_metadata: HashSet<PathBuf>,
    pub loading_git_statuses: HashSet<PathBuf>,
    pub loading_sizes: HashSet<PathBuf>,
    pub loading_ci_runs: HashSet<PathBuf>,
    pub loading_languages: HashSet<PathBuf>,
    pub loading_targets: HashSet<PathBuf>,
    pub focus: Focus,
    pub filter_mode: bool,
    pub project_query: String,
    pub ci_query: String,
    pub target_query: String,
    pub project_input: Input,
    pub ci_input: Input,
    pub target_input: Input,
    pub filtered_projects: Vec<usize>,
    pub filtered_ci_runs: Vec<usize>,
    pub filtered_targets: Vec<usize>,
    pub project_matcher: Nucleo<Project>,
    pub target_matcher: Nucleo<Target>,
    pub target_statuses: Vec<TargetStatus>,
    pub spinner: Spinner,
    pub spinner_state: SpinnerState,
    pub spinner_last_tick: Instant,
    pub git_status_scan_index: usize,
    pub pending_g: bool,
    pub projects_root: PathBuf,
    pub projects: Vec<Project>,
    pub recent_projects: RecentProjects,
    pub bookmarked_projects: BookmarkedProjects,
    pub background_tx: mpsc::UnboundedSender<BackgroundResult>,
    pub background_rx: mpsc::UnboundedReceiver<BackgroundResult>,
    pub background_limiter: Arc<Semaphore>,
    pub events: EventHandler,
}

impl Default for App {
    fn default() -> Self {
        let projects_root = projects_root();
        let recent_projects = load_recent_projects();
        let bookmarked_projects = load_bookmarked_projects();
        let mut projects = discover_projects(&projects_root);
        sort_projects(&mut projects, &recent_projects);
        let (background_tx, background_rx) = mpsc::unbounded_channel();

        let background_limiter = Arc::new(Semaphore::new(8));

        let mut app = Self {
            running: true,
            counter: 0,
            cursor: -1,
            target_cursor: -1,
            ci_cursor: -1,
            targets: Vec::new(),
            target_cache: HashMap::new(),
            metadata_cache: HashMap::new(),
            ci_runs_cache: HashMap::new(),
            languages_cache: HashMap::new(),
            git_status_cache: HashMap::new(),
            size_cache: HashMap::new(),
            loading_metadata: HashSet::new(),
            loading_git_statuses: HashSet::new(),
            loading_sizes: HashSet::new(),
            loading_ci_runs: HashSet::new(),
            loading_languages: HashSet::new(),
            loading_targets: HashSet::new(),
            focus: Focus::Projects,
            filter_mode: false,
            project_query: String::new(),
            ci_query: String::new(),
            target_query: String::new(),
            project_input: Input::default(),
            ci_input: Input::default(),
            target_input: Input::default(),
            filtered_projects: Vec::new(),
            filtered_ci_runs: Vec::new(),
            filtered_targets: Vec::new(),
            project_matcher: new_matcher(),
            target_matcher: new_matcher(),
            target_statuses: Vec::new(),
            spinner: Spinner::default().style(Style::default().fg(Color::Yellow)),
            spinner_state: SpinnerState::new(SpinnerType::Moon),
            spinner_last_tick: Instant::now(),
            git_status_scan_index: 0,
            pending_g: false,
            projects_root,
            projects,
            recent_projects,
            bookmarked_projects,
            background_tx,
            background_rx,
            background_limiter,
            events: EventHandler::new(),
        };

        app.rebuild_project_matcher();
        if !app.filtered_projects.is_empty() {
            app.cursor = 0;
            app.refresh_targets();
        }

        app.request_all_project_sizes();

        app
    }
}

impl App {
    pub fn is_bookmarked(&self, project: &Project) -> bool {
        let key = project.path.to_string_lossy();
        self.bookmarked_projects.entries.contains(key.as_ref())
    }

    pub fn project_last_opened(&self, project: &Project) -> String {
        let key = project.path.to_string_lossy();
        self.recent_projects
            .entries
            .get(key.as_ref())
            .copied()
            .map(format_timestamp)
            .unwrap_or_else(|| "—".to_string())
    }

    pub fn cached_project_size_bytes(&self, project: &Project) -> Option<u64> {
        self.size_cache.get(&project.path).copied()
    }

    pub fn target_status_for(&self, project: &Project, target: &Target) -> Option<&TargetStatus> {
        self.target_statuses.iter().find(|status| {
            status.project_path == project.path
                && status.target_name == target.name
                && status.target_kind == target.kind
        })
    }

    pub fn running_target_statuses(&self) -> impl Iterator<Item = &TargetStatus> {
        self.target_statuses.iter().filter(|status| {
            matches!(
                status.kind,
                TargetStatusKind::Building | TargetStatusKind::Running
            )
        })
    }

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

    pub fn new() -> Self {
        Self::default()
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        while self.running {
            terminal.draw(|frame| ui::render(frame, &mut self))?;
            match self.events.next().await? {
                Event::Tick => self.tick(),
                Event::Crossterm(event) => match event {
                    crossterm::event::Event::Key(key_event)
                        if key_event.kind == crossterm::event::KeyEventKind::Press =>
                    {
                        self.handle_key_events(key_event)?
                    }
                    _ => {}
                },
                Event::App(app_event) => match app_event {
                    AppEvent::Increment => self.increment_counter(),
                    AppEvent::Decrement => self.decrement_counter(),
                    AppEvent::Quit => self.quit(),
                },
            }
        }
        Ok(())
    }

    pub fn handle_key_events(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        if self.filter_mode {
            match key_event.code {
                KeyCode::Esc => self.cancel_filter_mode(),
                KeyCode::Enter => self.confirm_filter_mode(),
                KeyCode::Backspace
                | KeyCode::Char(_)
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Delete => self.handle_filter_input(key_event),
                _ => {}
            }
            return Ok(());
        }

        if self.pending_g {
            self.pending_g = false;
            match key_event.code {
                KeyCode::Char('g') => {
                    self.go_to_top();
                    return Ok(());
                }
                KeyCode::Char('e') => {
                    self.go_to_bottom();
                    return Ok(());
                }
                _ => {}
            }
        }

        if key_event.code == KeyCode::Backspace && self.active_filter_is_non_empty() {
            self.handle_filter_backspace();
            return Ok(());
        }

        match key_event.code {
            KeyCode::Esc => {
                self.pending_g = false;
                self.clear_all_filters()
            }
            KeyCode::Char('q') => self.events.send(AppEvent::Quit),
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Char('b') => self.toggle_selected_project_bookmark(),
            KeyCode::Char('c' | 'C') if key_event.modifiers == KeyModifiers::CONTROL => {
                self.events.send(AppEvent::Quit)
            }
            KeyCode::Char('/') => match self.focus {
                Focus::Projects => {
                    self.filter_mode = true;
                    self.cursor = if self.filtered_projects.is_empty() {
                        -1
                    } else {
                        0
                    };
                    self.refresh_targets();
                }
                Focus::CiRuns => {}
                Focus::Targets => {
                    self.filter_mode = true;
                    self.target_cursor = if self.filtered_targets.is_empty() {
                        -1
                    } else {
                        0
                    };
                }
            },
            KeyCode::Right | KeyCode::Char('l') => {
                self.focus = match self.focus {
                    Focus::Projects => Focus::Targets,
                    Focus::CiRuns => Focus::Projects,
                    Focus::Targets => Focus::Targets,
                };
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.focus = match self.focus {
                    Focus::Projects if self.project_ci_runs().is_some() => {
                        if self.ci_cursor < 0 {
                            self.ci_cursor = 0;
                        }
                        Focus::CiRuns
                    }
                    Focus::Projects => Focus::Projects,
                    Focus::CiRuns => Focus::CiRuns,
                    Focus::Targets => Focus::Projects,
                };
            }
            KeyCode::Char('j') | KeyCode::Down => match self.focus {
                Focus::Projects => self.select_next_project(),
                Focus::CiRuns => self.select_next_ci_run(),
                Focus::Targets => self.select_next_target(),
            },
            KeyCode::Char('k') | KeyCode::Up => match self.focus {
                Focus::Projects => self.select_previous_project(),
                Focus::CiRuns => self.select_previous_ci_run(),
                Focus::Targets => self.select_previous_target(),
            },
            KeyCode::Enter => match self.focus {
                Focus::Projects => {}
                Focus::CiRuns => self.open_selected_ci_run()?,
                Focus::Targets => self.run_selected_target()?,
            },
            KeyCode::Char('e') if self.focus == Focus::Targets => self.edit_selected_target()?,
            KeyCode::Char('O') => self.open_selected_project()?,
            _ => {}
        }
        Ok(())
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        let dt = now - self.spinner_last_tick;
        self.spinner_last_tick = now;
        self.spinner_state.tick(dt);
        self.refresh_target_status();

        let project_status = self.project_matcher.tick(0);
        if project_status.changed {
            self.sync_filtered_projects();
        }

        let target_status = self.target_matcher.tick(0);
        if target_status.changed {
            self.sync_filtered_targets();
        }

        self.drain_background_results();
        self.request_visible_project_data();
        self.request_visible_git_statuses();
        self.request_next_git_status_batch(4);
        self.request_current_project_metadata();
        self.request_current_project_ci_runs();
        self.request_current_project_languages();
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn increment_counter(&mut self) {
        self.counter = self.counter.saturating_add(1);
    }

    pub fn decrement_counter(&mut self) {
        self.counter = self.counter.saturating_sub(1);
    }

    pub fn select_next_project(&mut self) {
        if self.filtered_projects.is_empty() {
            return;
        }

        self.cursor = if self.cursor < 0 {
            0
        } else {
            (self.cursor + 1) % self.filtered_projects.len() as isize
        };
        self.refresh_targets();
    }

    pub fn go_to_top(&mut self) {
        match self.focus {
            Focus::Projects => {
                self.cursor = if self.filtered_projects.is_empty() {
                    -1
                } else {
                    0
                };
                self.refresh_targets();
            }
            Focus::CiRuns => {
                self.ci_cursor = if self.filtered_ci_runs.is_empty() {
                    -1
                } else {
                    0
                };
            }
            Focus::Targets => {
                self.target_cursor = if self.filtered_targets.is_empty() {
                    -1
                } else {
                    0
                };
            }
        }
    }

    pub fn go_to_bottom(&mut self) {
        match self.focus {
            Focus::Projects => {
                self.cursor = if self.filtered_projects.is_empty() {
                    -1
                } else {
                    self.filtered_projects.len() as isize - 1
                };
                self.refresh_targets();
            }
            Focus::CiRuns => {
                self.ci_cursor = if self.filtered_ci_runs.is_empty() {
                    -1
                } else {
                    self.filtered_ci_runs.len() as isize - 1
                };
            }
            Focus::Targets => {
                self.target_cursor = if self.filtered_targets.is_empty() {
                    -1
                } else {
                    self.filtered_targets.len() as isize - 1
                };
            }
        }
    }

    pub fn select_previous_project(&mut self) {
        if self.filtered_projects.is_empty() {
            return;
        }

        self.cursor = if self.cursor < 0 {
            self.filtered_projects.len() as isize - 1
        } else {
            (self.cursor - 1).rem_euclid(self.filtered_projects.len() as isize)
        };
        self.refresh_targets();
    }

    pub fn select_next_ci_run(&mut self) {
        if self.filtered_ci_runs.is_empty() {
            self.ci_cursor = -1;
            return;
        }

        self.ci_cursor = if self.ci_cursor < 0 {
            0
        } else {
            (self.ci_cursor + 1) % self.filtered_ci_runs.len() as isize
        };
    }

    pub fn select_next_target(&mut self) {
        if self.filtered_targets.is_empty() {
            return;
        }

        self.target_cursor = if self.target_cursor < 0 {
            0
        } else {
            (self.target_cursor + 1) % self.filtered_targets.len() as isize
        };
    }

    pub fn select_previous_ci_run(&mut self) {
        if self.filtered_ci_runs.is_empty() {
            self.ci_cursor = -1;
            return;
        }

        self.ci_cursor = if self.ci_cursor < 0 {
            self.filtered_ci_runs.len() as isize - 1
        } else {
            (self.ci_cursor - 1).rem_euclid(self.filtered_ci_runs.len() as isize)
        };
    }

    pub fn select_previous_target(&mut self) {
        if self.filtered_targets.is_empty() {
            return;
        }

        self.target_cursor = if self.target_cursor < 0 {
            self.filtered_targets.len() as isize - 1
        } else {
            (self.target_cursor - 1).rem_euclid(self.filtered_targets.len() as isize)
        };
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

    pub fn open_selected_project(&mut self) -> color_eyre::Result<()> {
        let Some(project) = self.current_project() else {
            return Ok(());
        };

        let project_path = project.path.clone();
        let command = format!(
            "zellij action new-tab && (cd {cwd} && env YAZI=false fish -c zellij_open_project) >/dev/null 2>&1 & zellij action go-to-previous-tab >/dev/null 2>&1",
            cwd = shell_escape_path(&project_path)
        );

        Command::new("sh")
            .arg("-c")
            .arg(command)
            // Detach stdio so zellij's informational output does not get rendered
            // into the dashboard TUI when opening a project in a new tab.
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        self.mark_project_opened(&project_path);
        Ok(())
    }

    pub fn run_selected_target(&mut self) -> color_eyre::Result<()> {
        let Some(project) = self.current_project() else {
            return Ok(());
        };
        let Some(target) = self.current_target() else {
            return Ok(());
        };

        let project_name = project.name.clone();
        let project_path = project.path.clone();
        let target_name = target.name.clone();
        let target_kind = target.kind.clone();
        let (status_path, log_path, pid_path) =
            target_runtime_paths(&project_path, &target_kind, &target_name);
        if let Some(parent) = status_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let run_command = match target_kind.as_str() {
            "bin" => format!("cargo run --bin {}", shell_escape_arg(&target_name)),
            "example" => format!("cargo run --example {}", shell_escape_arg(&target_name)),
            "test" => format!("cargo test --test {}", shell_escape_arg(&target_name)),
            "bench" => format!("cargo bench --bench {}", shell_escape_arg(&target_name)),
            _ => return Ok(()),
        };

        let pane_command = format!(
            "echo $$ > {pid}; echo building > {status}; {run} 2>&1 | tee {log}; rc=$?; echo done > {status}; exit $rc",
            pid = shell_escape_path(&pid_path),
            status = shell_escape_path(&status_path),
            log = shell_escape_path(&log_path),
            run = run_command,
        );

        let command = format!(
            "zellij action new-pane -f --close-on-exit --height 40 --width 140 --cwd {} -- sh -lc {}",
            shell_escape_path(&project_path),
            shell_escape_arg(&pane_command),
        );

        Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        self.target_statuses.retain(|status| {
            !(status.project_path == project_path
                && status.target_name == target_name
                && status.target_kind == target_kind)
        });
        self.target_statuses.push(TargetStatus {
            kind: TargetStatusKind::Building,
            project_name,
            project_path: project_path.clone(),
            target_name: target_name.clone(),
            target_kind: target_kind.clone(),
            profile: RunProfile::Debug,
            status_path,
            log_path,
            pid_path,
            pid: None,
            started_at: Some(Instant::now()),
            stats: ProcessStats::default(),
        });
        self.mark_project_opened(&project_path);
        Ok(())
    }

    pub fn edit_selected_target(&mut self) -> color_eyre::Result<()> {
        let Some(project) = self.current_project() else {
            return Ok(());
        };
        let Some(target) = self.current_target() else {
            return Ok(());
        };

        let target_path = project.path.join(&target.path);
        let command = format!(
            "zellij action new-pane -f --close-on-exit --height 40 --width 140 --cwd {} -- hx {}",
            shell_escape_path(&project.path),
            shell_escape_path(&target_path),
        );

        Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(())
    }

    pub fn current_project(&self) -> Option<&Project> {
        let visible_index = usize::try_from(self.cursor).ok()?;
        let project_index = *self.filtered_projects.get(visible_index)?;
        self.projects.get(project_index)
    }

    pub fn current_target(&self) -> Option<&Target> {
        let visible_index = usize::try_from(self.target_cursor).ok()?;
        let target_index = *self.filtered_targets.get(visible_index)?;
        self.targets.get(target_index)
    }

    pub fn current_target_description(&self) -> Option<&str> {
        if self.focus != Focus::Targets {
            return None;
        }
        self.current_target()?.description.as_deref()
    }

    pub fn targets_loading(&self) -> bool {
        self.current_project()
            .is_some_and(|project| self.loading_targets.contains(&project.path))
    }

    pub fn visible_projects(&self) -> impl Iterator<Item = &Project> {
        self.filtered_projects
            .iter()
            .filter_map(|&index| self.projects.get(index))
    }

    pub fn visible_targets(&self) -> impl Iterator<Item = &Target> {
        self.filtered_targets
            .iter()
            .filter_map(|&index| self.targets.get(index))
    }

    fn drain_background_results(&mut self) {
        while let Ok(result) = self.background_rx.try_recv() {
            match result {
                BackgroundResult::Metadata(path, metadata) => {
                    self.loading_metadata.remove(&path);
                    self.metadata_cache.insert(path, metadata);
                }
                BackgroundResult::GitStatus(path, git_status) => {
                    self.loading_git_statuses.remove(&path);
                    self.git_status_cache.insert(path, git_status);
                }
                BackgroundResult::Size(path, size) => {
                    self.loading_sizes.remove(&path);
                    self.size_cache.insert(path, size);
                }
                BackgroundResult::CiRuns(path, ci_runs) => {
                    let is_current_project = self
                        .current_project()
                        .is_some_and(|project| project.path == path);
                    self.loading_ci_runs.remove(&path);
                    self.ci_runs_cache.insert(path, ci_runs);
                    if is_current_project {
                        self.update_ci_filter();
                    }
                }
                BackgroundResult::Languages(path, languages) => {
                    self.loading_languages.remove(&path);
                    self.languages_cache.insert(path, languages);
                }
                BackgroundResult::Targets(path, targets) => {
                    let is_current_project = self
                        .current_project()
                        .is_some_and(|project| project.path == path);
                    self.loading_targets.remove(&path);
                    self.target_cache.insert(path, targets.clone());
                    if is_current_project {
                        self.targets = targets;
                        self.rebuild_target_matcher();
                    }
                }
            }
        }
    }

    fn request_current_project_metadata(&mut self) {
        let Some(project) = self.current_project() else {
            return;
        };
        self.request_metadata(project.path.clone());
    }

    fn request_current_project_ci_runs(&mut self) {
        let Some(project) = self.current_project() else {
            return;
        };
        self.request_ci_runs(project.path.clone());
    }

    fn request_current_project_languages(&mut self) {
        let Some(project) = self.current_project() else {
            return;
        };
        self.request_languages(project.path.clone());
    }

    fn request_visible_project_data(&mut self) {
        let visible_paths: Vec<_> = self
            .visible_projects()
            .take(8)
            .map(|project| project.path.clone())
            .collect();

        for path in visible_paths {
            self.request_size(path);
        }
    }

    fn request_visible_git_statuses(&mut self) {
        let visible_paths: Vec<_> = self
            .visible_projects()
            .take(16)
            .map(|project| project.path.clone())
            .collect();

        for path in visible_paths {
            self.request_git_status(path);
        }
    }

    fn request_all_project_sizes(&mut self) {
        let project_paths: Vec<_> = self
            .projects
            .iter()
            .map(|project| project.path.clone())
            .collect();
        for path in project_paths {
            self.request_size(path);
        }
    }

    fn request_next_git_status_batch(&mut self, batch_size: usize) {
        if self.projects.is_empty() {
            return;
        }

        for _ in 0..batch_size {
            let index = self.git_status_scan_index % self.projects.len();
            self.git_status_scan_index = self.git_status_scan_index.wrapping_add(1);
            if let Some(project) = self.projects.get(index) {
                self.request_git_status(project.path.clone());
            }
        }
    }

    fn request_metadata(&mut self, path: PathBuf) {
        if self.metadata_cache.contains_key(&path) || !self.loading_metadata.insert(path.clone()) {
            return;
        }

        let tx = self.background_tx.clone();
        let limiter = self.background_limiter.clone();
        tokio::spawn(async move {
            let Ok(_permit) = limiter.acquire_owned().await else {
                return;
            };
            let work_path = path.clone();
            let metadata =
                tokio::task::spawn_blocking(move || discover_project_metadata(&work_path))
                    .await
                    .unwrap_or_else(|_| ProjectMetadataSummary::default());
            let _ = tx.send(BackgroundResult::Metadata(path, metadata));
        });
    }

    fn request_git_status(&mut self, path: PathBuf) {
        if self.git_status_cache.contains_key(&path)
            || !self.loading_git_statuses.insert(path.clone())
        {
            return;
        }

        let tx = self.background_tx.clone();
        let limiter = self.background_limiter.clone();
        tokio::spawn(async move {
            let Ok(_permit) = limiter.acquire_owned().await else {
                return;
            };
            let work_path = path.clone();
            let git_status = tokio::task::spawn_blocking(move || git_status(&work_path))
                .await
                .unwrap_or_else(|_| "UNTRACKED".to_string());
            let _ = tx.send(BackgroundResult::GitStatus(path, git_status));
        });
    }

    fn request_size(&mut self, path: PathBuf) {
        if self.size_cache.contains_key(&path) || !self.loading_sizes.insert(path.clone()) {
            return;
        }

        let tx = self.background_tx.clone();
        let limiter = self.background_limiter.clone();
        tokio::spawn(async move {
            let Ok(_permit) = limiter.acquire_owned().await else {
                return;
            };
            let work_path = path.clone();
            let size = tokio::task::spawn_blocking(move || directory_size(&work_path))
                .await
                .unwrap_or(0);
            let _ = tx.send(BackgroundResult::Size(path, size));
        });
    }

    fn request_ci_runs(&mut self, path: PathBuf) {
        if self.ci_runs_cache.contains_key(&path) || !self.loading_ci_runs.insert(path.clone()) {
            return;
        }

        let tx = self.background_tx.clone();
        let limiter = self.background_limiter.clone();
        tokio::spawn(async move {
            let Ok(_permit) = limiter.acquire_owned().await else {
                return;
            };
            let work_path = path.clone();
            let ci_runs = tokio::task::spawn_blocking(move || discover_ci_runs(&work_path))
                .await
                .ok()
                .flatten();
            let _ = tx.send(BackgroundResult::CiRuns(path, ci_runs));
        });
    }

    fn request_languages(&mut self, path: PathBuf) {
        if self.languages_cache.contains_key(&path) || !self.loading_languages.insert(path.clone())
        {
            return;
        }

        let tx = self.background_tx.clone();
        let limiter = self.background_limiter.clone();
        tokio::spawn(async move {
            let Ok(_permit) = limiter.acquire_owned().await else {
                return;
            };
            let work_path = path.clone();
            let languages = tokio::task::spawn_blocking(move || discover_languages(&work_path))
                .await
                .ok()
                .flatten();
            let _ = tx.send(BackgroundResult::Languages(path, languages));
        });
    }

    fn request_targets(&mut self, path: PathBuf) {
        if self.target_cache.contains_key(&path) || !self.loading_targets.insert(path.clone()) {
            return;
        }

        let tx = self.background_tx.clone();
        let limiter = self.background_limiter.clone();
        tokio::spawn(async move {
            let Ok(_permit) = limiter.acquire_owned().await else {
                return;
            };
            let work_path = path.clone();
            let targets = tokio::task::spawn_blocking(move || discover_targets(&work_path))
                .await
                .unwrap_or_default();
            let _ = tx.send(BackgroundResult::Targets(path, targets));
        });
    }

    fn handle_filter_input(&mut self, key_event: KeyEvent) {
        let request = match key_event.code {
            KeyCode::Backspace => Some(InputRequest::DeletePrevChar),
            KeyCode::Left => Some(InputRequest::GoToPrevChar),
            KeyCode::Right => Some(InputRequest::GoToNextChar),
            KeyCode::Home => Some(InputRequest::GoToStart),
            KeyCode::End => Some(InputRequest::GoToEnd),
            KeyCode::Delete => Some(InputRequest::DeleteNextChar),
            KeyCode::Char(c)
                if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
            {
                Some(InputRequest::InsertChar(c))
            }
            _ => None,
        };

        let Some(request) = request else {
            return;
        };

        self.apply_filter_request(request);
    }

    fn handle_filter_backspace(&mut self) {
        self.apply_filter_request(InputRequest::DeletePrevChar);
    }

    fn active_filter_is_non_empty(&self) -> bool {
        match self.focus {
            Focus::Projects => !self.project_query.is_empty(),
            Focus::CiRuns => !self.ci_query.is_empty(),
            Focus::Targets => !self.target_query.is_empty(),
        }
    }

    fn apply_filter_request(&mut self, request: InputRequest) {
        match self.focus {
            Focus::Projects => {
                self.project_input.handle(request);
                self.project_query = self.project_input.value().to_string();
                self.update_project_filter();
            }
            Focus::CiRuns => {
                self.ci_input.handle(request);
                self.ci_query = self.ci_input.value().to_string();
                self.update_ci_filter();
            }
            Focus::Targets => {
                self.target_input.handle(request);
                self.target_query = self.target_input.value().to_string();
                self.update_target_filter();
            }
        }
    }

    fn confirm_filter_mode(&mut self) {
        self.filter_mode = false;
        match self.focus {
            Focus::Projects => {
                self.cursor = if self.filtered_projects.is_empty() {
                    -1
                } else {
                    0
                };
                self.refresh_targets();
            }
            Focus::CiRuns => {
                self.ci_cursor = if self.filtered_ci_runs.is_empty() {
                    -1
                } else {
                    0
                };
            }
            Focus::Targets => {
                self.target_cursor = if self.filtered_targets.is_empty() {
                    -1
                } else {
                    0
                };
            }
        }
    }

    fn cancel_filter_mode(&mut self) {
        self.filter_mode = false;
        self.clear_all_filters();
    }

    fn clear_all_filters(&mut self) {
        self.project_input.reset();
        self.project_query.clear();
        self.update_project_filter();
        self.cursor = if self.filtered_projects.is_empty() {
            -1
        } else {
            0
        };

        self.ci_input.reset();
        self.ci_query.clear();
        self.update_ci_filter();

        self.target_input.reset();
        self.target_query.clear();
        self.update_target_filter();
        self.target_cursor = if self.filtered_targets.is_empty() {
            -1
        } else {
            0
        };
        self.ci_cursor = if self.filtered_ci_runs.is_empty() {
            -1
        } else {
            0
        };

        self.refresh_targets();
    }

    fn refresh_targets(&mut self) {
        let Some(project) = self.current_project() else {
            self.targets.clear();
            self.filtered_targets.clear();
            self.target_cursor = -1;
            self.filtered_ci_runs.clear();
            self.ci_cursor = -1;
            return;
        };

        let path = project.path.clone();
        if let Some(targets) = self.target_cache.get(&path) {
            self.targets = targets.clone();
            self.rebuild_target_matcher();
        } else {
            self.targets.clear();
            self.filtered_targets.clear();
            self.target_cursor = -1;
            self.request_targets(path);
        }
        self.update_ci_filter();
    }

    fn refresh_target_status(&mut self) {
        for status in &mut self.target_statuses {
            if let Ok(contents) = fs::read_to_string(&status.status_path) {
                match contents.trim() {
                    "building" => status.kind = TargetStatusKind::Building,
                    "running" => {
                        if status.kind != TargetStatusKind::Running {
                            status.kind = TargetStatusKind::Running;
                            status.started_at = Some(Instant::now());
                        }
                    }
                    "done" => {
                        status.kind = TargetStatusKind::Running;
                        status.started_at.get_or_insert_with(Instant::now);
                        status.pid = None;
                    }
                    _ => {}
                }
            }

            if status.pid.is_none() {
                status.pid = fs::read_to_string(&status.pid_path)
                    .ok()
                    .and_then(|contents| contents.trim().parse::<u32>().ok());
            }

            status.stats = read_process_stats(status.pid);
        }

        self.target_statuses.retain(|status| {
            fs::read_to_string(&status.status_path)
                .map(|contents| contents.trim() != "done")
                .unwrap_or(true)
        });
    }

    fn toggle_selected_project_bookmark(&mut self) {
        let Some(project) = self.current_project() else {
            return;
        };

        let key = project.path.to_string_lossy().to_string();
        if !self.bookmarked_projects.entries.insert(key.clone()) {
            self.bookmarked_projects.entries.remove(&key);
        }
        let _ = save_bookmarked_projects(&self.bookmarked_projects);
    }

    fn mark_project_opened(&mut self, project_path: &Path) {
        let key = project_path.to_string_lossy().into_owned();
        self.recent_projects
            .entries
            .insert(key, unix_timestamp_now());
        let _ = save_recent_projects(&self.recent_projects);

        let selected_path = self.current_project().map(|project| project.path.clone());
        sort_projects(&mut self.projects, &self.recent_projects);
        self.rebuild_project_matcher();

        if let Some(selected_path) = selected_path {
            if let Some(index) = self.filtered_projects.iter().position(|&project_index| {
                self.projects
                    .get(project_index)
                    .is_some_and(|project| project.path == selected_path)
            }) {
                self.cursor = index as isize;
            }
        }
    }

    fn rebuild_project_matcher(&mut self) {
        self.project_matcher.restart(true);
        let injector = self.project_matcher.injector();
        for project in self.projects.clone() {
            injector.push(project, |project, cols| {
                cols[0] = Utf32String::from(
                    format!("{} {}", project.name, project.path.display()).as_str(),
                );
            });
        }
        self.update_project_filter();
    }

    fn rebuild_target_matcher(&mut self) {
        self.target_matcher.restart(true);
        let injector = self.target_matcher.injector();
        for target in self.targets.clone() {
            injector.push(target, |target, cols| {
                cols[0] = Utf32String::from(
                    format!("{} {} {}", target.kind, target.name, target.path).as_str(),
                );
            });
        }
        self.update_target_filter();
    }

    fn update_project_filter(&mut self) {
        self.project_matcher.pattern.reparse(
            0,
            &self.project_query,
            CaseMatching::Smart,
            Normalization::Smart,
            false,
        );
        let _ = self.project_matcher.tick(10);
        self.sync_filtered_projects();
    }

    fn update_ci_filter(&mut self) {
        let query = self.ci_query.to_ascii_lowercase();
        self.filtered_ci_runs = self
            .project_ci_runs()
            .map(|ci| {
                ci.runs
                    .iter()
                    .enumerate()
                    .filter(|(_, run)| {
                        query.is_empty()
                            || run.branch.to_ascii_lowercase().contains(&query)
                            || run.title.to_ascii_lowercase().contains(&query)
                            || run.status.to_ascii_lowercase().contains(&query)
                            || run.created_at.to_ascii_lowercase().contains(&query)
                    })
                    .map(|(index, _)| index)
                    .collect()
            })
            .unwrap_or_default();
        self.ci_cursor = if self.filtered_ci_runs.is_empty() {
            -1
        } else {
            self.ci_cursor
                .clamp(0, self.filtered_ci_runs.len() as isize - 1)
        };
    }

    fn update_target_filter(&mut self) {
        self.target_matcher.pattern.reparse(
            0,
            &self.target_query,
            CaseMatching::Smart,
            Normalization::Smart,
            false,
        );
        let _ = self.target_matcher.tick(10);
        self.sync_filtered_targets();
    }

    fn sync_filtered_projects(&mut self) {
        self.filtered_projects = self
            .project_matcher
            .snapshot()
            .matched_items(..)
            .map(|item| item.data.clone())
            .filter_map(|project| {
                self.projects
                    .iter()
                    .position(|candidate| candidate.path == project.path)
            })
            .collect();

        if self.filtered_projects.is_empty() {
            self.cursor = -1;
            self.targets.clear();
            self.filtered_targets.clear();
            self.target_cursor = -1;
        } else {
            self.cursor = self
                .cursor
                .clamp(0, self.filtered_projects.len() as isize - 1);
        }
    }

    fn sync_filtered_targets(&mut self) {
        self.filtered_targets = self
            .target_matcher
            .snapshot()
            .matched_items(..)
            .map(|item| item.data.clone())
            .filter_map(|target| {
                self.targets.iter().position(|candidate| {
                    candidate.kind == target.kind
                        && candidate.name == target.name
                        && candidate.path == target.path
                })
            })
            .collect();

        self.target_cursor = if self.filtered_targets.is_empty() {
            -1
        } else {
            self.target_cursor
                .clamp(0, self.filtered_targets.len() as isize - 1)
        };
    }
}

fn new_matcher<T: Sync + Send + 'static>() -> Nucleo<T> {
    Nucleo::new(Config::DEFAULT, Arc::new(|| {}), Some(1), 1)
}

fn dashboard_state_dir() -> PathBuf {
    env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| expand_tilde(PathBuf::from("~/.local/state")))
        .join("dashboard")
}

fn bookmarked_projects_path() -> PathBuf {
    dashboard_state_dir().join("bookmarked-projects.json")
}

fn load_bookmarked_projects() -> BookmarkedProjects {
    let path = bookmarked_projects_path();
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_bookmarked_projects(bookmarked_projects: &BookmarkedProjects) -> std::io::Result<()> {
    let path = bookmarked_projects_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(bookmarked_projects)?;
    fs::write(path, content)
}

fn recent_projects_path() -> PathBuf {
    dashboard_state_dir().join("recent-projects.json")
}

fn load_recent_projects() -> RecentProjects {
    let path = recent_projects_path();
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_recent_projects(recent_projects: &RecentProjects) -> std::io::Result<()> {
    let path = recent_projects_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(recent_projects)?;
    fs::write(path, content)
}

fn target_runtime_paths(
    project_path: &Path,
    target_kind: &str,
    target_name: &str,
) -> (PathBuf, PathBuf, PathBuf) {
    let key = sanitize_runtime_key(&format!(
        "{}__{}__{}",
        project_path.display(),
        target_kind,
        target_name
    ));
    let dir = dashboard_state_dir().join("targets");
    (
        dir.join(format!("{key}.status")),
        dir.join(format!("{key}.log")),
        dir.join(format!("{key}.pid")),
    )
}

fn sanitize_runtime_key(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn format_timestamp(timestamp: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp as i64, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|| "—".to_string())
}

fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn sort_projects(projects: &mut [Project], recent_projects: &RecentProjects) {
    projects.sort_by(|a, b| {
        let a_key = a.path.to_string_lossy();
        let b_key = b.path.to_string_lossy();
        let a_recent = recent_projects.entries.get(a_key.as_ref()).copied();
        let b_recent = recent_projects.entries.get(b_key.as_ref()).copied();

        b_recent
            .cmp(&a_recent)
            .then_with(|| match (a_recent, b_recent) {
                (None, None) => b.has_git_dir.cmp(&a.has_git_dir),
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.name.cmp(&b.name))
    });
}

fn projects_root() -> PathBuf {
    env::var("PROJECTS_DIR")
        .map(PathBuf::from)
        .map(expand_tilde)
        .unwrap_or_else(|_| expand_tilde(PathBuf::from("~/code")))
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str == "~" || path_str.starts_with("~/") {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home).join(path_str.trim_start_matches("~/"));
        }
    }
    path
}

fn discover_projects(root: &Path) -> Vec<Project> {
    let mut projects = Vec::new();
    let mut seen = HashSet::new();
    visit_dirs(root, &mut projects, &mut seen);
    projects
}

fn visit_dirs(dir: &Path, projects: &mut Vec<Project>, seen: &mut HashSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    let mut has_git_dir = false;
    let mut has_cargo_toml = false;
    let mut child_dirs = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                if name == ".git" {
                    has_git_dir = true;
                    continue;
                }

                if should_skip_project_scan_dir(name) {
                    continue;
                }
            }

            child_dirs.push(path);
            continue;
        }

        if path.file_name().is_some_and(|name| name == "Cargo.toml") {
            has_cargo_toml = true;
        }
    }

    let is_workspace = has_cargo_toml && is_workspace_root(dir);

    if (has_git_dir || has_cargo_toml) && seen.insert(dir.to_path_buf()) {
        if let Some(name) = dir.file_name().and_then(|name| name.to_str()) {
            projects.push(Project {
                name: name.to_string(),
                path: dir.to_path_buf(),
                has_git_dir,
                is_workspace,
            });
        }
    }

    if is_workspace {
        return;
    }

    for child in child_dirs {
        visit_dirs(&child, projects, seen);
    }
}

fn is_workspace_root(dir: &Path) -> bool {
    fs::read_to_string(dir.join("Cargo.toml"))
        .ok()
        .is_some_and(|cargo_toml| cargo_toml.contains("[workspace]"))
}

fn should_skip_project_scan_dir(name: &str) -> bool {
    matches!(
        name,
        "target"
            | "node_modules"
            | ".direnv"
            | ".devenv"
            | ".jj"
            | ".venv"
            | "venv"
            | "dist"
            | "build"
            | ".next"
            | ".turbo"
    )
}

fn directory_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };

    if metadata.is_file() {
        return metadata.len();
    }

    if !metadata.is_dir() {
        return 0;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };

    entries
        .flatten()
        .map(|entry| directory_size(&entry.path()))
        .sum()
}

fn read_process_stats(pid: Option<u32>) -> ProcessStats {
    let Some(pid) = pid else {
        return ProcessStats::default();
    };

    let output = Command::new("ps")
        .args(["-o", "%cpu=,rss=", "-p", &pid.to_string()])
        .output();

    let Ok(output) = output else {
        return ProcessStats::default();
    };

    if !output.status.success() {
        return ProcessStats::default();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut parts = stdout.split_whitespace();
    let cpu_percent = parts.next().and_then(|value| value.parse::<f32>().ok());
    let memory_bytes = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|kb| kb * 1024);

    ProcessStats {
        cpu_percent,
        memory_bytes,
    }
}

fn shell_escape_path(path: &Path) -> String {
    shell_escape_arg(&path.display().to_string())
}

fn shell_escape_arg(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn discover_project_metadata(project_path: &Path) -> ProjectMetadataSummary {
    let metadata = MetadataCommand::new()
        .current_dir(project_path)
        .no_deps()
        .exec()
        .ok();

    let workspace_root = metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path() == project_path)
        .unwrap_or(false);
    let package = metadata.as_ref().and_then(|metadata| metadata.root_package());

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

fn current_git_branch(project_path: &Path) -> Option<String> {
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

fn git_status(project_path: &Path) -> String {
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

fn discover_ci_runs(project_path: &Path) -> Option<CiRunsData> {
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

fn github_repo_url(project_path: &Path) -> Option<String> {
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

fn normalize_github_remote(remote: &str) -> Option<String> {
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

fn discover_languages(project_path: &Path) -> Option<LanguagesData> {
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

fn shell_url_encode(value: &str) -> String {
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

fn discover_targets(project_path: &Path) -> Vec<Target> {
    let Ok(metadata) = MetadataCommand::new()
        .current_dir(project_path)
        .no_deps()
        .exec()
    else {
        return Vec::new();
    };

    let mut targets = metadata
        .packages
        .iter()
        .flat_map(|package| {
            let manifest_descriptions =
                load_target_descriptions(package.manifest_path.as_std_path());
            package.targets.iter().filter_map(move |target| {
                select_target_kind(target).map(|kind| Target {
                    kind: kind.to_string(),
                    name: target.name.clone(),
                    path: target
                        .src_path
                        .strip_prefix(project_path)
                        .map(|path| path.to_string())
                        .unwrap_or_else(|_| target.src_path.to_string()),
                    description: manifest_descriptions
                        .get(&(kind.to_string(), target.name.clone()))
                        .cloned()
                        .flatten(),
                })
            })
        })
        .collect::<Vec<_>>();

    targets.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.kind.cmp(&b.kind))
    });
    targets
}

fn load_target_descriptions(
    manifest_path: &Path,
) -> HashMap<(String, String), Option<String>> {
    let Ok(cargo_toml) = fs::read_to_string(manifest_path) else {
        return HashMap::new();
    };
    let Ok(value) = toml::from_str::<toml::Value>(&cargo_toml) else {
        return HashMap::new();
    };

    let mut descriptions = HashMap::new();
    for (manifest_key, target_kind) in [
        ("bin", "bin"),
        ("example", "example"),
        ("test", "test"),
        ("bench", "bench"),
    ] {
        if let Some(items) = value.get(manifest_key).and_then(|items| items.as_array()) {
            for item in items {
                let Some(table) = item.as_table() else {
                    continue;
                };
                let Some(name) = table.get("name").and_then(|name| name.as_str()) else {
                    continue;
                };
                let description = table
                    .get("description")
                    .and_then(|description| description.as_str())
                    .map(str::trim)
                    .filter(|description| !description.is_empty())
                    .map(ToOwned::to_owned);
                descriptions.insert((target_kind.to_string(), name.to_string()), description);
            }
        }

        let metadata_examples = value
            .get("package")
            .and_then(|package| package.get("metadata"))
            .and_then(|metadata| metadata.get(manifest_key))
            .and_then(|targets| targets.as_table());

        if let Some(targets) = metadata_examples {
            for (name, target_metadata) in targets {
                let description = target_metadata
                    .get("description")
                    .and_then(|description| description.as_str())
                    .map(str::trim)
                    .filter(|description| !description.is_empty())
                    .map(ToOwned::to_owned);
                if description.is_some() {
                    descriptions.insert((target_kind.to_string(), name.to_string()), description);
                }
            }
        }
    }

    descriptions
}

fn select_target_kind(target: &CargoTarget) -> Option<&'static str> {
    if target.is_bin() {
        Some("bin")
    } else if target.is_example() {
        Some("example")
    } else if target.is_test() {
        Some("test")
    } else if target.is_bench() {
        Some("bench")
    } else {
        None
    }
}

