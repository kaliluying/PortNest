mod platform;

use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listener {
    pub port: u16,
    pub addresses: Vec<String>,
    pub scope: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub name: String,
    pub root: String,
    pub marker: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessRecord {
    pub id: String,
    pub pid: u32,
    pub uid: Option<u32>,
    pub name: String,
    pub executable: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<u64>,
    pub project: Option<Project>,
    pub category: String,
    pub protection_reason: Option<String>,
    pub can_close: bool,
    pub listeners: Vec<Listener>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub processes: Vec<ProcessRecord>,
    pub scanned_at: u64,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosePlan {
    pub token: String,
    pub process: ProcessRecord,
    pub expires_at: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseOutcome {
    pub status: String,
    pub pid: u32,
    pub remaining_ports: Vec<u16>,
    pub message: String,
    pub force_token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub pid: u32,
    pub uid: u32,
    pub real_uid: u32,
    pub saved_uid: u32,
    pub executable: String,
    pub cwd: String,
    pub started_micros: u64,
    pub flags: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseKind {
    Graceful,
    Force,
}

struct StoredPlan {
    identity: Identity,
    listeners: Vec<Listener>,
    expires: Instant,
    kind: CloseKind,
}

#[derive(Default)]
struct CloseState {
    plans: HashMap<String, StoredPlan>,
    active: HashSet<u32>,
}

#[derive(Default)]
pub struct Core {
    state: Mutex<CloseState>,
}

// No lock is held while scanning or waiting for a process to exit.
struct ActiveClose<'a> {
    core: &'a Core,
    pid: u32,
}

impl Drop for ActiveClose<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.core.state.lock() {
            state.active.remove(&self.pid);
        }
    }
}

const PLAN_LIFETIME: Duration = Duration::from_secs(30);
const CLOSE_WAIT: Duration = Duration::from_secs(5);

impl Core {
    pub fn scan(&self) -> Result<Snapshot, String> {
        let raw = platform::scan_listeners(None)?;
        let managed = platform::managed_pids();
        let mut limitations = vec![
            "仅显示当前用户无需管理员权限可读取的 TCP 监听；系统权限可能限制其他进程的信息。"
                .into(),
            "项目归属根据工作目录及最近项目标记推断；绑定范围不代表网络可达性。".into(),
        ];
        if managed.is_err() {
            limitations.push("无法读取 launchd 托管状态，关闭功能暂不可用。".into());
        }
        let mut processes = Vec::new();
        for process in raw {
            let identity = platform::identity(process.pid).ok().flatten();
            let reason = protection_reason(identity.as_ref(), managed.as_ref().ok());
            processes.push(make_record(process, identity.as_ref(), reason));
        }
        processes.sort_by_key(|p| (!p.can_close, p.listeners[0].port, p.pid));
        Ok(Snapshot {
            processes,
            scanned_at: now_ms(),
            limitations,
        })
    }

    pub fn prepare_close(&self, pid: u32) -> Result<ClosePlan, String> {
        let _active = self.claim(pid)?;
        let before = required_identity(pid)?;
        let process = platform::scan_listeners(Some(pid))?
            .into_iter()
            .find(|p| p.pid == pid)
            .ok_or("该进程已不再监听，请刷新列表。")?;
        let identity = required_identity(pid)?;
        if before != identity {
            return Err("进程身份在扫描期间变化，请刷新列表并重新确认。".into());
        }
        let managed = platform::managed_pids()?;
        let reason = protection_reason(Some(&identity), Some(&managed));
        if let Some(reason) = reason {
            return Err(format!("无法关闭：{reason}"));
        }
        let record = make_record(process, Some(&identity), None);
        let token = self.store_plan(identity, record.listeners.clone(), CloseKind::Graceful)?;
        Ok(ClosePlan {
            token,
            process: record,
            expires_at: now_ms() + PLAN_LIFETIME.as_millis() as u64,
        })
    }

    pub fn execute_close(&self, token: &str) -> Result<CloseOutcome, String> {
        self.close(token, CloseKind::Graceful)
    }

    pub fn force_close(&self, token: &str) -> Result<CloseOutcome, String> {
        self.close(token, CloseKind::Force)
    }

    fn claim(&self, pid: u32) -> Result<ActiveClose<'_>, String> {
        let mut state = self.state.lock().map_err(|_| "关闭状态不可用。")?;
        if !state.active.insert(pid) {
            return Err("该进程已有关闭操作正在进行，请等待结果。".into());
        }
        Ok(ActiveClose { core: self, pid })
    }

    fn store_plan(
        &self,
        identity: Identity,
        listeners: Vec<Listener>,
        kind: CloseKind,
    ) -> Result<String, String> {
        let mut state = self.state.lock().map_err(|_| "关闭状态不可用。")?;
        state
            .plans
            .retain(|_, p| p.expires > Instant::now() && p.identity.pid != identity.pid);
        if state.plans.len() >= 128 {
            return Err("待确认的关闭操作过多，请稍后重试。".into());
        }
        let token = uuid::Uuid::new_v4().to_string();
        state.plans.insert(
            token.clone(),
            StoredPlan {
                identity,
                listeners,
                expires: Instant::now() + PLAN_LIFETIME,
                kind,
            },
        );
        Ok(token)
    }

    fn take_plan(&self, token: &str, kind: CloseKind) -> Result<StoredPlan, String> {
        let mut state = self.state.lock().map_err(|_| "关闭状态不可用。")?;
        let plan = state
            .plans
            .remove(token)
            .ok_or("关闭确认已失效或已使用，请重新查看详情。")?;
        if plan.expires <= Instant::now() {
            return Err("关闭确认已过期，请重新查看详情。".into());
        }
        if plan.kind != kind {
            return Err("关闭确认类型不匹配，请重新确认操作。".into());
        }
        Ok(plan)
    }

    fn close(&self, token: &str, kind: CloseKind) -> Result<CloseOutcome, String> {
        let plan = self.take_plan(token, kind)?;
        let pid = plan.identity.pid;
        let _active = self.claim(pid)?;
        let current = required_identity(pid)?;
        validate_identity(&plan.identity, &current)?;
        let managed = platform::managed_pids()?;
        if let Some(reason) = protection_reason(Some(&current), Some(&managed)) {
            return Err(format!("无法关闭：{reason}"));
        }
        let listeners = platform::scan_listeners(Some(pid))?
            .into_iter()
            .find(|p| p.pid == pid)
            .map(|p| p.listeners)
            .unwrap_or_default();
        validate_impact(&plan.listeners, &listeners)?;
        validate_identity(&plan.identity, &required_identity(pid)?)?;
        if plan.expires <= Instant::now() {
            return Err("关闭确认在核验期间过期，未发送信号；请重新确认。".into());
        }
        platform::send_signal(pid, kind == CloseKind::Force)?;

        let deadline = Instant::now() + CLOSE_WAIT;
        loop {
            let identity = platform::identity(pid).map_err(|e| after_signal(&e))?;
            if original_exited(&plan.identity, identity.as_ref()).map_err(|e| after_signal(&e))? {
                return Ok(closed(pid, "原进程已退出；刷新列表可查看当前监听服务。"));
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let listeners = platform::scan_listeners(Some(pid))
            .map_err(|e| after_signal(&e))?
            .into_iter()
            .find(|p| p.pid == pid)
            .map(|p| p.listeners)
            .unwrap_or_default();
        let identity = platform::identity(pid).map_err(|e| after_signal(&e))?;
        if original_exited(&plan.identity, identity.as_ref()).map_err(|e| after_signal(&e))? {
            return Ok(closed(pid, "原进程已退出；PID 已变化时不会继续操作。"));
        }
        if listeners.is_empty() {
            return Ok(closed(pid, "该进程已停止 TCP 监听；进程可能仍在执行清理。"));
        }
        let ports = listeners.iter().map(|l| l.port).collect();
        let force_token = if kind == CloseKind::Graceful {
            // A force confirmation can only cover the impact the user just confirmed.
            validate_impact(&plan.listeners, &listeners).map_err(|e| after_signal(&e))?;
            Some(
                self.store_plan(plan.identity, listeners, CloseKind::Force)
                    .map_err(|e| after_signal(&e))?,
            )
        } else {
            None
        };
        Ok(CloseOutcome {
            status: "stillListening".into(),
            pid,
            remaining_ports: ports,
            message: if kind == CloseKind::Graceful {
                "已请求正常退出，等待 5 秒后仍在监听；可再次确认强制关闭。"
            } else {
                "已发送强制关闭信号，等待 5 秒后仍在监听，请刷新检查。"
            }
            .into(),
            force_token,
        })
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn required_identity(pid: u32) -> Result<Identity, String> {
    platform::identity(pid)?.ok_or_else(|| "原进程已退出，请刷新列表。".into())
}

fn validate_identity(expected: &Identity, current: &Identity) -> Result<(), String> {
    if expected != current {
        return Err("进程身份已变化，未发送信号；请刷新并重新确认。".into());
    }
    Ok(())
}

fn original_exited(expected: &Identity, current: Option<&Identity>) -> Result<bool, String> {
    let Some(current) = current else {
        return Ok(true);
    };
    if current.started_micros != expected.started_micros {
        return Ok(true);
    }
    validate_identity(expected, current)?;
    Ok(false)
}

fn validate_impact(expected: &[Listener], current: &[Listener]) -> Result<(), String> {
    if expected != current {
        return Err("监听端口或绑定地址已变化，未发送新的信号；请刷新并重新确认。".into());
    }
    Ok(())
}

fn after_signal(error: &str) -> String {
    format!("信号已经发送，但无法确认最终状态：{error} 请刷新检查，勿将此错误视为关闭成功。")
}

fn closed(pid: u32, message: &str) -> CloseOutcome {
    CloseOutcome {
        status: "closed".into(),
        pid,
        remaining_ports: vec![],
        message: message.into(),
        force_token: None,
    }
}

fn protection_reason(
    identity: Option<&Identity>,
    managed: Option<&HashSet<u32>>,
) -> Option<String> {
    let Some(i) = identity else {
        return Some("无法完整核验进程身份。".into());
    };
    if i.pid <= 1 || i.pid == std::process::id() {
        return Some("系统进程或 PortNest 自身受保护。".into());
    }
    if i.uid == 0 || i.real_uid == 0 || i.saved_uid == 0 || i.flags & 0x2001 != 0 {
        return Some("系统或特权进程受保护。".into());
    }
    if i.uid != platform::current_uid() || i.real_uid != i.uid || i.saved_uid != i.uid {
        return Some("其他用户或身份发生权限切换的进程受保护。".into());
    }
    if i.executable.is_empty() || i.cwd.is_empty() || i.started_micros == 0 {
        return Some("无法完整核验进程身份。".into());
    }
    if Path::new(&i.executable)
        .components()
        .any(|p| p.as_os_str().to_string_lossy().ends_with(".app"))
    {
        return Some("App 及其应用包内的辅助进程受保护。".into());
    }
    if ["/System/", "/usr/libexec/", "/usr/sbin/", "/sbin/"]
        .iter()
        .any(|p| i.executable.starts_with(p))
    {
        return Some("系统服务受保护。".into());
    }
    match managed {
        None => Some("无法核验 launchd 托管状态。".into()),
        Some(pids) if pids.contains(&i.pid) => {
            Some("launchd 托管进程受保护，退出后可能自动重启。".into())
        }
        _ => None,
    }
}

fn make_record(
    raw: platform::RawProcess,
    identity: Option<&Identity>,
    reason: Option<String>,
) -> ProcessRecord {
    let project = identity.and_then(|i| project_for(&i.cwd));
    let can_close = reason.is_none();
    ProcessRecord {
        id: identity
            .map(|i| format!("{}:{}", i.pid, i.started_micros))
            .unwrap_or_else(|| format!("{}:unknown", raw.pid)),
        pid: raw.pid,
        uid: identity.map(|i| i.uid).or(raw.uid),
        name: raw.name,
        executable: identity.map(|i| i.executable.clone()),
        cwd: identity.map(|i| i.cwd.clone()),
        started_at: identity.map(|i| i.started_micros / 1000),
        category: if !can_close {
            "protected"
        } else if project.is_some() {
            "development"
        } else {
            "unknown"
        }
        .into(),
        project,
        protection_reason: reason,
        can_close,
        listeners: raw.listeners,
    }
}

fn project_for(cwd: &str) -> Option<Project> {
    let directory = Path::new(cwd);
    for root in directory.ancestors() {
        for marker in [
            "package.json",
            "Cargo.toml",
            "pyproject.toml",
            "go.mod",
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "Package.swift",
            "composer.json",
            "Gemfile",
            ".git",
        ] {
            if root.join(marker).exists() {
                return Some(Project {
                    name: root.file_name()?.to_string_lossy().into_owned(),
                    root: root.to_string_lossy().into_owned(),
                    marker: marker.into(),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
