use crate::{Identity, Listener};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub(crate) struct RawProcess {
    pub pid: u32,
    pub uid: Option<u32>,
    pub name: String,
    pub listeners: Vec<Listener>,
}

#[cfg(target_os = "macos")]
pub(crate) use macos::{current_uid, identity, managed_pids, scan_listeners, send_signal};

#[cfg(not(target_os = "macos"))]
pub(crate) fn current_uid() -> u32 {
    u32::MAX
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn identity(_: u32) -> Result<Option<Identity>, String> {
    Err("首版目前仅支持 macOS。".into())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn managed_pids() -> Result<HashSet<u32>, String> {
    Err("首版目前仅支持 macOS。".into())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn scan_listeners(_: Option<u32>) -> Result<Vec<RawProcess>, String> {
    Err("首版目前仅支持 macOS。".into())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn send_signal(_: u32, _: bool) -> Result<(), String> {
    Err("首版目前仅支持 macOS。".into())
}

fn parse_listeners(bytes: &[u8]) -> Result<Vec<RawProcess>, String> {
    let mut processes = Vec::new();
    let mut current: Option<RawProcess> = None;
    let mut listeners: BTreeMap<u16, BTreeSet<String>> = BTreeMap::new();
    let finish = |current: &mut Option<RawProcess>,
                  listeners: &mut BTreeMap<u16, BTreeSet<String>>,
                  processes: &mut Vec<RawProcess>| {
        if let Some(mut process) = current.take() {
            process.listeners = std::mem::take(listeners)
                .into_iter()
                .map(|(port, addresses)| {
                    let addresses: Vec<String> = addresses.into_iter().collect();
                    let local = addresses.iter().all(|a| is_loopback(a));
                    Listener {
                        port,
                        addresses,
                        scope: if local { "local" } else { "lan" }.into(),
                    }
                })
                .collect();
            if !process.listeners.is_empty() {
                processes.push(process);
            }
        }
    };
    for raw in bytes.split(|b| *b == 0) {
        let raw = raw.strip_prefix(b"\n").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let value =
            std::str::from_utf8(&raw[1..]).map_err(|_| "监听扫描输出包含无法解析的文字。")?;
        match raw[0] {
            b'p' => {
                finish(&mut current, &mut listeners, &mut processes);
                let pid = value
                    .parse::<u32>()
                    .map_err(|_| "监听扫描返回无效的 PID。")?;
                if pid == 0 || pid > i32::MAX as u32 {
                    return Err("监听扫描返回无效的 PID。".into());
                }
                current = Some(RawProcess {
                    pid,
                    uid: None,
                    name: format!("进程 {pid}"),
                    listeners: Vec::new(),
                });
            }
            b'c' => {
                if let Some(p) = current.as_mut() {
                    p.name = value.into();
                }
            }
            b'u' => {
                if let Some(p) = current.as_mut() {
                    p.uid = value.parse().ok();
                }
            }
            b'n' => {
                if current.is_none() {
                    return Err("监听扫描返回缺失进程的端口。".into());
                }
                let (address, port) = value
                    .rsplit_once(':')
                    .ok_or("监听扫描返回无法识别的地址。")?;
                let port = port
                    .parse::<u16>()
                    .map_err(|_| "监听扫描返回无效的端口。")?;
                if port == 0 {
                    return Err("监听扫描返回无效的端口。".into());
                }
                let address = address
                    .strip_prefix('[')
                    .and_then(|a| a.strip_suffix(']'))
                    .unwrap_or(address);
                if address != "*"
                    && address
                        .split('%')
                        .next()
                        .unwrap_or("")
                        .parse::<std::net::IpAddr>()
                        .is_err()
                {
                    return Err("监听扫描返回非数字绑定地址。".into());
                }
                listeners.entry(port).or_default().insert(address.into());
            }
            b'f' => {}
            _ => return Err("监听扫描返回未预期的字段，请刷新重试。".into()),
        }
    }
    finish(&mut current, &mut listeners, &mut processes);
    Ok(processes)
}

fn is_loopback(address: &str) -> bool {
    address
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

fn managed_descendants(seeds: &HashSet<u32>, parents: &[(u32, u32)]) -> HashSet<u32> {
    let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for (pid, parent) in parents {
        children.entry(*parent).or_default().push(*pid);
    }
    let mut managed = seeds.clone();
    let mut pending: Vec<u32> = seeds.iter().copied().collect();
    while let Some(parent) = pending.pop() {
        for child in children.get(&parent).into_iter().flatten() {
            if managed.insert(*child) {
                pending.push(*child);
            }
        }
    }
    managed
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::{
        io::Read,
        process::{Command, ExitStatus, Stdio},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };

    pub(crate) fn current_uid() -> u32 {
        unsafe { libc::geteuid() }
    }

    pub(crate) fn scan_listeners(pid: Option<u32>) -> Result<Vec<RawProcess>, String> {
        let mut args = vec![
            "-nP".into(),
            "-a".into(),
            "-iTCP".into(),
            "-sTCP:LISTEN".into(),
            "-F0pcufn".into(),
        ];
        if let Some(pid) = pid {
            args.extend(["-p".into(), pid.to_string()]);
        }
        let (status, stdout, stderr) = run_bounded("/usr/sbin/lsof", &args)?;
        if !stderr.is_empty() {
            return Err("系统监听扫描返回警告或错误，结果不完整；请刷新重试。".into());
        }
        if status.code() == Some(1) && stdout.is_empty() {
            return Ok(vec![]);
        }
        if !status.success() {
            return Err("系统监听扫描失败，未将失败结果视为空列表。".into());
        }
        parse_listeners(&stdout)
    }

    pub(crate) fn managed_pids() -> Result<HashSet<u32>, String> {
        let (status, stdout, stderr) = run_bounded("/bin/launchctl", &["list".into()])?;
        if !status.success() || !stderr.is_empty() {
            return Err("无法读取 launchd 托管进程，未发送信号。".into());
        }
        let text = std::str::from_utf8(&stdout).map_err(|_| "无法解析 launchd 托管状态。")?;
        if !text.starts_with("PID\tStatus\tLabel\n") {
            return Err("launchd 托管状态格式不可识别，未发送信号。".into());
        }
        let mut direct = HashSet::new();
        let mut seeds = HashSet::new();
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() != 3 {
                return Err("launchd 托管状态格式不完整，未发送信号。".into());
            }
            let Ok(pid) = fields[0].parse::<u32>() else {
                continue;
            };
            direct.insert(pid);
            // App ancestry includes Terminal/IDE-launched servers; it is not a service job.
            if !fields[2].starts_with("application.") {
                if let Ok(path) = executable_path(pid) {
                    if !std::path::Path::new(&path)
                        .components()
                        .any(|p| p.as_os_str().to_string_lossy().ends_with(".app"))
                    {
                        seeds.insert(pid);
                    }
                }
            }
        }
        let (status, stdout, stderr) =
            run_bounded("/bin/ps", &["-axo".into(), "pid=,ppid=".into()])?;
        if !status.success() || !stderr.is_empty() {
            return Err("无法核验托管进程祖先，未发送信号。".into());
        }
        let text = std::str::from_utf8(&stdout).map_err(|_| "无法解析进程祖先关系。")?;
        let mut parents = Vec::new();
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let pid = fields
                .next()
                .and_then(|p| p.parse::<u32>().ok())
                .ok_or("无法解析进程祖先 PID。")?;
            let parent = fields
                .next()
                .and_then(|p| p.parse::<u32>().ok())
                .ok_or("无法解析父进程 PID。")?;
            if fields.next().is_some() {
                return Err("进程祖先关系返回未预期的字段。".into());
            }
            parents.push((pid, parent));
        }
        direct.extend(managed_descendants(&seeds, &parents));
        Ok(direct)
    }

    pub(crate) fn identity(pid: u32) -> Result<Option<Identity>, String> {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err("无效的进程 PID。".into());
        }
        let Some(before) = bsd_info(pid)? else {
            return Ok(None);
        };
        let executable = match executable_path(pid) {
            Ok(path) => path,
            Err(_) => {
                return process_read_error(pid, "无法读取可执行文件路径，身份不足以安全关闭。")
            }
        };
        let mut vnode: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>();
        let count = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&mut vnode as *mut libc::proc_vnodepathinfo).cast(),
                size as i32,
            )
        };
        if count != size as i32 {
            return process_read_error(pid, "无法读取进程工作目录，身份不足以安全关闭。");
        }
        let cwd_bytes = unsafe {
            std::slice::from_raw_parts(vnode.pvi_cdir.vip_path.as_ptr().cast::<u8>(), 1024)
        };
        let cwd = c_string(cwd_bytes)?;
        let Some(after) = bsd_info(pid)? else {
            return Ok(None);
        };
        if before.pbi_pid != after.pbi_pid
            || before.pbi_start_tvsec != after.pbi_start_tvsec
            || before.pbi_start_tvusec != after.pbi_start_tvusec
            || before.pbi_uid != after.pbi_uid
            || before.pbi_ruid != after.pbi_ruid
            || before.pbi_svuid != after.pbi_svuid
            || before.pbi_flags & 0x2001 != after.pbi_flags & 0x2001
        {
            return Err("读取期间进程身份发生变化，请刷新重试。".into());
        }
        Ok(Some(Identity {
            pid,
            uid: after.pbi_uid,
            real_uid: after.pbi_ruid,
            saved_uid: after.pbi_svuid,
            executable,
            cwd,
            started_micros: after
                .pbi_start_tvsec
                .saturating_mul(1_000_000)
                .saturating_add(after.pbi_start_tvusec),
            // Exit and scheduling flags are transient; retain system/set-privilege flags.
            flags: after.pbi_flags & 0x2001,
        }))
    }

    fn bsd_info(pid: u32) -> Result<Option<libc::proc_bsdinfo>, String> {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>();
        let count = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size as i32,
            )
        };
        if count != size as i32 {
            return process_read_error(pid, "无法核验进程身份，请刷新检查权限。");
        }
        if info.pbi_status == 5 {
            return Ok(None);
        } // SZOMB: the exited child is awaiting collection.
        if info.pbi_pid != pid {
            return Err("系统返回的进程身份不匹配。".into());
        }
        Ok(Some(info))
    }

    fn process_read_error<T>(pid: u32, message: &str) -> Result<Option<T>, String> {
        let result = unsafe { libc::kill(pid as i32, 0) };
        if result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        // libproc excludes exited children that their parent has not collected yet.
        let (status, stdout, stderr) = run_bounded(
            "/bin/ps",
            &["-p".into(), pid.to_string(), "-o".into(), "stat=".into()],
        )?;
        if stderr.is_empty() {
            if status.code() == Some(1) && stdout.is_empty() {
                return Ok(None);
            }
            if status.success()
                && std::str::from_utf8(&stdout).is_ok_and(|s| s.trim().starts_with('Z'))
            {
                return Ok(None);
            }
        }
        Err(message.into())
    }

    fn c_string(bytes: &[u8]) -> Result<String, String> {
        let end = bytes
            .iter()
            .position(|b| *b == 0)
            .ok_or("系统路径信息不完整，无法安全核验身份。")?;
        std::str::from_utf8(&bytes[..end])
            .map(str::to_owned)
            .map_err(|_| "系统路径不是有效 UTF-8，无法安全核验身份。".into())
    }

    fn executable_path(pid: u32) -> Result<String, String> {
        let mut path = [0u8; 4096];
        let count =
            unsafe { libc::proc_pidpath(pid as i32, path.as_mut_ptr().cast(), path.len() as u32) };
        if count <= 0 {
            return Err("无法读取可执行文件路径。".into());
        }
        c_string(&path)
    }

    pub(crate) fn send_signal(pid: u32, force: bool) -> Result<(), String> {
        if pid <= 1 || pid > i32::MAX as u32 {
            return Err("无效的目标 PID，未发送信号。".into());
        }
        let result = unsafe {
            libc::kill(
                pid as i32,
                if force { libc::SIGKILL } else { libc::SIGTERM },
            )
        };
        if result != 0 {
            return Err(format!(
                "无法发送关闭信号：{}。",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    fn run_bounded(path: &str, args: &[String]) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), String> {
        const LIMIT: usize = 4 * 1024 * 1024;
        let mut child = Command::new(path)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "无法启动系统扫描工具。")?;
        let overflow = Arc::new(AtomicBool::new(false));
        let reader = |stream: Box<dyn Read + Send>, overflow: Arc<AtomicBool>| {
            std::thread::spawn(move || {
                let mut output = Vec::new();
                let result = stream.take((LIMIT + 1) as u64).read_to_end(&mut output);
                if output.len() > LIMIT {
                    overflow.store(true, Ordering::Relaxed);
                }
                (result, output)
            })
        };
        let stdout = reader(Box::new(child.stdout.take().unwrap()), overflow.clone());
        let stderr = reader(Box::new(child.stderr.take().unwrap()), overflow.clone());
        let deadline = Instant::now() + Duration::from_secs(3);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {}
                Err(_) => break Err("无法读取系统扫描工具状态。"),
            }
            if overflow.load(Ordering::Relaxed) {
                break Err("系统扫描输出超过安全上限，请缩小范围后重试。");
            }
            if Instant::now() >= deadline {
                break Err("系统扫描超时，请刷新重试。");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        if status.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let stdout = stdout.join().map_err(|_| "无法读取系统扫描结果。")?;
        let stderr = stderr.join().map_err(|_| "无法读取系统扫描错误状态。")?;
        let status = status.map_err(str::to_owned)?;
        if overflow.load(Ordering::Relaxed) {
            return Err("系统扫描输出超过安全上限。".into());
        }
        stdout.0.map_err(|_| "系统扫描结果读取失败。")?;
        stderr.0.map_err(|_| "系统扫描错误状态读取失败。")?;
        Ok((status, stdout.1, stderr.1))
    }

    #[cfg(test)]
    mod command_tests {
        use super::*;

        #[test]
        fn scan_command_output_limit_and_timeout_are_errors() {
            let oversized = run_bounded("/usr/bin/yes", &[]).unwrap_err();
            assert!(oversized.contains("上限"));
            let started = Instant::now();
            let timed_out = run_bounded("/bin/sleep", &["10".into()]).unwrap_err();
            assert!(timed_out.contains("超时"));
            assert!(started.elapsed() < Duration::from_secs(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_groups_ipv4_ipv6_and_wildcard_with_numeric_addresses() {
        let raw = b"p42\0cnode\0u501\0\nf3\0n127.0.0.1:3000\0\nf4\0n[::1]:3000\0\nf5\0n*:4000\0\nf6\0n127.0.0.1:3000\0\n";
        let p = parse_listeners(raw).unwrap().remove(0);
        assert_eq!(p.pid, 42);
        assert_eq!(p.listeners.len(), 2);
        assert_eq!(p.listeners[0].addresses, ["127.0.0.1", "::1"]);
        assert_eq!(p.listeners[0].scope, "local");
        assert_eq!(p.listeners[1].scope, "lan");
    }

    #[test]
    fn bad_output_is_an_error_not_empty_success() {
        for raw in [
            b"p-1\0".as_slice(),
            b"p42\0n*:65536\0",
            b"p42\0ninvalid:3000\0",
            b"n*:3000\0",
            b"p42\0zunexpected\0",
        ] {
            assert!(parse_listeners(raw).is_err());
        }
        assert!(parse_listeners(b"").unwrap().is_empty());
    }

    #[test]
    fn managed_service_descendants_are_protected_without_protecting_orphans_or_terminal_children() {
        // 10 is a verified non-App launchd service; Terminal (20) is not a service seed.
        let parents = [
            (10, 1),
            (11, 10),
            (12, 11),
            (20, 1),
            (21, 20),
            (22, 21),
            (30, 1),
            (31, 30),
        ];
        let managed = managed_descendants(&HashSet::from([10]), &parents);
        assert_eq!(managed, HashSet::from([10, 11, 12]));
        assert!(!managed.contains(&22));
        assert!(!managed.contains(&30));
        assert!(!managed.contains(&31));
        assert_eq!(
            managed_descendants(&HashSet::from([10]), &[(10, 11), (11, 10)]),
            HashSet::from([10, 11])
        );
    }
}
