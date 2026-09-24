use super::projects::Project;
use super::targets::{Target, discover_targets};
use super::{App, Focus};
use portable_pty::{CommandBuilder as PtyCommandBuilder, PtySize, native_pty_system};
use std::collections::HashSet;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetStatusKind {
    Building,
    Running,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunProfile {
    Debug,
    Release,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProcessStats {
    pub(crate) cpu_percent: Option<f32>,
    pub(crate) memory_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct TargetStatus {
    pub(crate) kind: TargetStatusKind,
    pub(crate) project_name: String,
    pub(crate) project_path: std::path::PathBuf,
    pub(crate) target_name: String,
    pub(crate) target_kind: String,
    pub(crate) key: String,
    pub(crate) profile: RunProfile,
    pub(crate) pid: Option<u32>,
    pub(crate) started_at: Option<Instant>,
    pub(crate) stats: ProcessStats,
}

pub(crate) struct RunningSession {
    pub(crate) key: String,
    pub(crate) parser: Arc<Mutex<vt100::Parser>>,
    pub(crate) master: Box<dyn portable_pty::MasterPty + Send>,
    pub(crate) writer: Box<dyn Write + Send>,
    pub(crate) child: Box<dyn portable_pty::Child + Send>,
}

pub(super) fn shell_escape_path(path: &Path) -> String {
    shell_escape_arg(&path.display().to_string())
}
pub(super) fn shell_escape_arg(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

impl App {
    pub fn target_status_for(&self, project: &Project, target: &Target) -> Option<&TargetStatus> {
        self.target_statuses.iter().find(|status| {
            status.project_path == project.path
                && status.target_name == target.name
                && status.target_kind == target.kind
                && matches!(
                    status.kind,
                    TargetStatusKind::Building
                        | TargetStatusKind::Running
                        | TargetStatusKind::Failed
                )
        })
    }

    pub fn running_target_statuses(&self) -> impl Iterator<Item = &TargetStatus> {
        self.target_statuses.iter().filter(|status| {
            matches!(
                status.kind,
                TargetStatusKind::Building | TargetStatusKind::Running | TargetStatusKind::Failed
            )
        })
    }

    pub fn current_running_target_status(&self) -> Option<&TargetStatus> {
        usize::try_from(self.running_cursor)
            .ok()
            .and_then(|index| self.running_target_statuses().nth(index))
    }

    pub fn selected_running_terminal_parser(&self) -> Option<Arc<Mutex<vt100::Parser>>> {
        let key = self.current_running_target_status()?.key.clone();
        self.running_sessions
            .iter()
            .find(|session| session.key == key)
            .map(|session| Arc::clone(&session.parser))
    }

    pub fn resize_selected_running_terminal(&mut self, width: u16, height: u16) {
        let Some(key) = self
            .current_running_target_status()
            .map(|status| status.key.clone())
        else {
            return;
        };
        let Some(session) = self
            .running_sessions
            .iter_mut()
            .find(|session| session.key == key)
        else {
            return;
        };
        let rows = height.max(1);
        let cols = width.max(1);
        let _ = session.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut parser) = session.parser.lock() {
            parser.screen_mut().set_size(rows, cols);
        }
    }
}
pub(super) fn target_runtime_key(
    project_path: &Path,
    target_kind: &str,
    target_name: &str,
) -> String {
    sanitize_runtime_key(&format!(
        "{}__{}__{}",
        project_path.display(),
        target_kind,
        target_name
    ))
}

fn sanitize_runtime_key(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

pub(super) fn read_process_stats(pid: Option<u32>) -> ProcessStats {
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
impl App {
    pub fn rerun_selected_running_target(&mut self) -> color_eyre::Result<()> {
        let Some(status) = self.current_running_target_status().cloned() else {
            return Ok(());
        };

        self.close_selected_running_target()?;

        let target = self
            .target_cache
            .get(&status.project_path)
            .and_then(|targets| {
                targets
                    .iter()
                    .find(|target| {
                        target.name == status.target_name && target.kind == status.target_kind
                    })
                    .cloned()
            })
            .or_else(|| {
                let targets = discover_targets(&status.project_path);
                let target = targets.iter().find(|target| {
                    target.name == status.target_name && target.kind == status.target_kind
                })?;
                Some(target.clone())
            });

        let Some(target) = target else {
            return Ok(());
        };

        self.start_target_run(status.project_name, status.project_path, target, false)
    }

    pub fn close_selected_running_target(&mut self) -> color_eyre::Result<()> {
        let Some(key) = self
            .current_running_target_status()
            .map(|status| status.key.clone())
        else {
            return Ok(());
        };

        if let Some(session) = self
            .running_sessions
            .iter_mut()
            .find(|session| session.key == key)
        {
            let _ = session.child.kill();
        }

        self.running_sessions.retain(|session| session.key != key);
        self.target_statuses.retain(|status| status.key != key);

        let running_count = self.running_target_statuses().count() as isize;
        self.running_cursor = if running_count == 0 {
            -1
        } else {
            self.running_cursor.clamp(0, running_count - 1)
        };

        if running_count == 0 && self.focus == Focus::RunningTargets {
            self.focus = if self.project_ci_runs().is_some() {
                Focus::CiRuns
            } else {
                Focus::Projects
            };
        }

        Ok(())
    }

    pub fn run_selected_target(&mut self) -> color_eyre::Result<()> {
        let Some(project) = self.current_project() else {
            return Ok(());
        };
        let Some(target) = self.current_target().cloned() else {
            return Ok(());
        };

        self.start_target_run(project.name.clone(), project.path.clone(), target, true)
    }

    fn start_target_run(
        &mut self,
        project_name: String,
        project_path: PathBuf,
        target: Target,
        reuse_existing: bool,
    ) -> color_eyre::Result<()> {
        let target_name = target.name.clone();
        let target_kind = target.kind.clone();
        let key = target_runtime_key(&project_path, &target_kind, &target_name);

        if reuse_existing {
            if let Some(index) = self.target_statuses.iter().position(|status| {
                status.project_path == project_path
                    && status.target_name == target_name
                    && status.target_kind == target_kind
                    && matches!(
                        status.kind,
                        TargetStatusKind::Building
                            | TargetStatusKind::Running
                            | TargetStatusKind::Failed
                    )
            }) {
                self.running_cursor = self
                    .target_statuses
                    .iter()
                    .take(index + 1)
                    .filter(|status| {
                        matches!(
                            status.kind,
                            TargetStatusKind::Building
                                | TargetStatusKind::Running
                                | TargetStatusKind::Failed
                        )
                    })
                    .count() as isize
                    - 1;
                self.focus = Focus::RunningTargets;
                return Ok(());
            }
        }

        let package_flag = format!(" -p {}", shell_escape_arg(&target.package_name));
        let feature_flags = if target.required_features.is_empty() {
            String::new()
        } else {
            format!(
                " --features {}",
                shell_escape_arg(&target.required_features.join(","))
            )
        };

        let run_command = match target_kind.as_str() {
            "bin" => format!(
                "cargo --color always run{} --bin {}{}",
                package_flag,
                shell_escape_arg(&target_name),
                feature_flags
            ),
            "example" => format!(
                "cargo --color always run{} --example {}{}",
                package_flag,
                shell_escape_arg(&target_name),
                feature_flags
            ),
            "test" => format!(
                "cargo --color always test{} --test {}{}",
                package_flag,
                shell_escape_arg(&target_name),
                feature_flags
            ),
            "bench" => format!(
                "cargo --color always bench{} --bench {}{}",
                package_flag,
                shell_escape_arg(&target_name),
                feature_flags
            ),
            _ => return Ok(()),
        };

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| color_eyre::eyre::eyre!(error.to_string()))?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(24, 120, 10_000)));
        let mut cmd = PtyCommandBuilder::new("bash");
        cmd.cwd(&project_path);
        cmd.arg("-lc");
        cmd.arg(&run_command);
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|error| color_eyre::eyre::eyre!(error.to_string()))?;
        let pid = child.process_id();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| color_eyre::eyre::eyre!(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| color_eyre::eyre::eyre!(error.to_string()))?;
        let parser_for_thread = Arc::clone(&parser);
        thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut parser) = parser_for_thread.lock() {
                            parser.process(&buffer[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        self.running_sessions.push(RunningSession {
            key: key.clone(),
            parser,
            master: pair.master,
            writer,
            child,
        });
        self.target_statuses.push(TargetStatus {
            kind: TargetStatusKind::Building,
            project_name,
            project_path: project_path.clone(),
            target_name: target_name.clone(),
            target_kind: target_kind.clone(),
            key,
            profile: RunProfile::Debug,
            pid,
            started_at: Some(Instant::now()),
            stats: read_process_stats(pid),
        });
        self.running_cursor = self.running_target_statuses().count() as isize - 1;
        self.focus = Focus::RunningTargets;
        self.sync_project_selection_to_running_target();
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
            "zellij action new-pane -f --close-on-exit --height 50 --width 140 --cwd {} -- hx {}",
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
    pub(super) fn refresh_target_status(&mut self) {
        let mut finished = HashSet::new();

        for session in &mut self.running_sessions {
            if let Ok(Some(exit_status)) = session.child.try_wait() {
                if exit_status.success() {
                    finished.insert(session.key.clone());
                } else if let Some(status) = self
                    .target_statuses
                    .iter_mut()
                    .find(|status| status.key == session.key)
                {
                    status.kind = TargetStatusKind::Failed;
                    status.pid = None;
                    status.stats = ProcessStats::default();
                }
            }
        }

        for status in &mut self.target_statuses {
            if finished.contains(&status.key) || status.kind == TargetStatusKind::Failed {
                continue;
            }
            if status
                .started_at
                .is_some_and(|started_at| started_at.elapsed().as_millis() > 750)
            {
                status.kind = TargetStatusKind::Running;
            }
            status.pid = self
                .running_sessions
                .iter()
                .find(|session| session.key == status.key)
                .and_then(|session| session.child.process_id())
                .or(status.pid);
            status.stats = read_process_stats(status.pid);
        }

        self.running_sessions
            .retain(|session| !finished.contains(&session.key));
        self.target_statuses
            .retain(|status| !finished.contains(&status.key));

        let running_count = self.running_target_statuses().count() as isize;
        self.running_cursor = if running_count == 0 {
            -1
        } else {
            self.running_cursor.clamp(0, running_count - 1)
        };
    }
}
