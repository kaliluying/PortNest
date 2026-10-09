use super::*;

fn identity() -> Identity {
    let uid = platform::current_uid();
    Identity {
        pid: 50_000,
        uid,
        real_uid: uid,
        saved_uid: uid,
        executable: "/opt/homebrew/bin/node".into(),
        cwd: "/tmp/portnest-project".into(),
        started_micros: 123_456_789,
        flags: 0,
    }
}

fn listeners() -> Vec<Listener> {
    vec![Listener {
        port: 3000,
        addresses: vec!["127.0.0.1".into()],
        scope: "local".into(),
    }]
}

#[test]
fn protects_system_other_users_apps_self_and_incomplete_identity() {
    let managed = HashSet::new();
    let check = |i: &Identity| protection_reason(Some(i), Some(&managed)).is_some();
    let original = identity();
    assert!(!check(&original));
    let mut variants = vec![];
    let mut i = original.clone();
    i.uid = 0;
    variants.push(i);
    let mut i = original.clone();
    i.uid += 1;
    variants.push(i);
    let mut i = original.clone();
    i.saved_uid = 0;
    variants.push(i);
    let mut i = original.clone();
    i.flags = 1;
    variants.push(i);
    let mut i = original.clone();
    i.pid = std::process::id();
    variants.push(i);
    let mut i = original.clone();
    i.executable = "/Applications/Example.app/Contents/MacOS/helper".into();
    variants.push(i);
    let mut i = original.clone();
    i.executable = "/System/Library/example".into();
    variants.push(i);
    let mut i = original.clone();
    i.started_micros = 0;
    variants.push(i);
    let mut i = original.clone();
    i.cwd.clear();
    variants.push(i);
    for variant in variants {
        assert!(check(&variant), "{variant:?}");
    }
    assert!(protection_reason(None, Some(&managed)).is_some());
    assert!(protection_reason(Some(&original), None).is_some());
    assert!(protection_reason(Some(&original), Some(&HashSet::from([original.pid]))).is_some());
}

#[test]
fn identity_reuse_exec_uid_cwd_and_submillisecond_changes_are_rejected() {
    let original = identity();
    let mut changes = vec![];
    let mut i = original.clone();
    i.started_micros += 1;
    changes.push(i);
    let mut i = original.clone();
    i.executable.push('2');
    changes.push(i);
    let mut i = original.clone();
    i.uid += 1;
    changes.push(i);
    let mut i = original.clone();
    i.cwd.push('2');
    changes.push(i);
    let mut i = original.clone();
    i.flags = 1;
    changes.push(i);
    for change in changes {
        assert!(validate_identity(&original, &change).is_err());
    }
    assert!(validate_identity(&original, &original).is_ok());
}

#[test]
fn impact_checks_every_port_and_binding_address() {
    let original = listeners();
    let mut added = original.clone();
    added.push(Listener {
        port: 3001,
        addresses: vec!["::1".into()],
        scope: "local".into(),
    });
    let mut rebound = original.clone();
    rebound[0].addresses = vec!["*".into()];
    assert!(validate_impact(&original, &added).is_err());
    assert!(validate_impact(&original, &rebound).is_err());
    assert!(validate_impact(&original, &[]).is_err());
}

#[test]
fn post_signal_changes_do_not_claim_success_or_erase_uncertainty() {
    let original = identity();
    assert!(original_exited(&original, None).unwrap());
    let mut reused = original.clone();
    reused.started_micros += 1;
    assert!(original_exited(&original, Some(&reused)).unwrap());
    let mut changed = original.clone();
    changed.cwd.push('2');
    assert!(original_exited(&original, Some(&changed)).is_err());
    assert!(!original_exited(&original, Some(&original)).unwrap());
    let message = after_signal("扫描超时");
    assert!(message.contains("信号已经发送"));
    assert!(message.contains("勿将此错误视为关闭成功"));
}

#[test]
fn plans_are_server_owned_single_use_expiring_and_kind_checked() {
    let core = Core::default();
    assert!(core.execute_close("forged").is_err());
    assert!(core.force_close("forged").is_err());
    let token = core
        .store_plan(identity(), listeners(), CloseKind::Graceful)
        .unwrap();
    assert!(core.force_close(&token).is_err());
    assert!(core.execute_close(&token).is_err());
    let token = core
        .store_plan(identity(), listeners(), CloseKind::Graceful)
        .unwrap();
    core.state
        .lock()
        .unwrap()
        .plans
        .get_mut(&token)
        .unwrap()
        .expires = Instant::now() - Duration::from_secs(1);
    assert!(core.execute_close(&token).unwrap_err().contains("过期"));
    let old = core
        .store_plan(identity(), listeners(), CloseKind::Graceful)
        .unwrap();
    let new = core
        .store_plan(identity(), listeners(), CloseKind::Graceful)
        .unwrap();
    assert!(core.take_plan(&old, CloseKind::Graceful).is_err());
    assert!(core.take_plan(&new, CloseKind::Graceful).is_ok());
    assert!(core.take_plan(&new, CloseKind::Graceful).is_err());
}

#[test]
fn concurrent_operations_on_same_pid_are_blocked_and_released() {
    let core = Core::default();
    let active = core.claim(42).unwrap();
    assert!(core.claim(42).is_err());
    assert!(core.claim(43).is_ok());
    drop(active);
    assert!(core.claim(42).is_ok());
}

#[test]
fn unknown_project_does_not_disable_verified_current_user_close() {
    let raw = platform::RawProcess {
        pid: 50_000,
        uid: None,
        name: "node".into(),
        listeners: listeners(),
    };
    let identity = identity();
    let record = make_record(raw, Some(&identity), None);
    assert_eq!(record.category, "unknown");
    assert!(record.can_close);
    let value = serde_json::to_value(record).unwrap();
    assert!(value.get("canClose").is_some());
    assert!(value.get("startedAt").is_some());
    assert!(value.get("protectionReason").is_some());
}

#[cfg(target_os = "macos")]
mod real {
    use super::*;
    use std::{
        fs,
        net::TcpListener,
        process::{Child, Command, Stdio},
        thread,
    };

    struct Service {
        child: Child,
        root: std::path::PathBuf,
        ports: Vec<u16>,
    }
    impl Drop for Service {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    // The fixture only serves when explicitly spawned by its owning regression test.
    #[test]
    fn listener_fixture() {
        let Ok(ready) = std::env::var("PORTNEST_TEST_READY") else {
            return;
        };
        if std::env::var("PORTNEST_TEST_IGNORE_TERM").is_ok() {
            unsafe {
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
            }
        }
        let first = TcpListener::bind("127.0.0.1:0").unwrap();
        let second = TcpListener::bind("[::1]:0").unwrap();
        let ports = [
            first.local_addr().unwrap().port(),
            second.local_addr().unwrap().port(),
        ];
        fs::write(ready, serde_json::to_vec(&ports).unwrap()).unwrap();
        loop {
            thread::sleep(Duration::from_millis(100));
            if Path::new("add-port").exists() {
                let third = TcpListener::bind("127.0.0.1:0").unwrap();
                fs::write("added", third.local_addr().unwrap().port().to_string()).unwrap();
                loop {
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }

    fn service(ignore_term: bool) -> Service {
        let root = std::env::temp_dir().join(format!("portnest-core-e2e-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        fs::write(root.join("package.json"), "{}").unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "tests::real::listener_fixture", "--nocapture"])
            .current_dir(&root)
            .env("PORTNEST_TEST_READY", root.join("ready.json"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if ignore_term {
            command.env("PORTNEST_TEST_IGNORE_TERM", "1");
        }
        let child = command.spawn().unwrap();
        let mut service = Service {
            child,
            root,
            ports: vec![],
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !service.root.join("ready.json").exists() {
            assert!(Instant::now() < deadline, "fixture did not start");
            thread::sleep(Duration::from_millis(20));
        }
        service.ports =
            serde_json::from_slice(&fs::read(service.root.join("ready.json")).unwrap()).unwrap();
        service
    }

    #[test]
    fn real_ipv4_ipv6_multiport_scan_project_and_graceful_close() {
        let mut service = service(false);
        let core = Core::default();
        let snapshot = core.scan().unwrap();
        let record = snapshot
            .processes
            .iter()
            .find(|p| p.pid == service.child.id())
            .unwrap();
        assert!(record.can_close, "{:?}", record.protection_reason);
        assert_eq!(record.listeners.len(), 2);
        assert_eq!(
            record.project.as_ref().unwrap().root,
            service.root.to_str().unwrap()
        );
        assert_eq!(record.category, "development");
        let plan = core.prepare_close(service.child.id()).unwrap();
        assert_eq!(plan.process.listeners.len(), 2);
        let outcome = core.execute_close(&plan.token).unwrap();
        assert_eq!(outcome.status, "closed");
        // The child is deliberately not reaped until identity verifies its exited state.
        assert!(platform::identity(service.child.id()).unwrap().is_none());
        assert!(core.execute_close(&plan.token).is_err());
        service.child.wait().unwrap();
        assert!(!core
            .scan()
            .unwrap()
            .processes
            .iter()
            .any(|p| p.pid == service.child.id()));
        for port in &service.ports {
            assert!(std::net::TcpStream::connect(("127.0.0.1", *port)).is_err());
            assert!(std::net::TcpStream::connect(("::1", *port)).is_err());
        }
    }

    #[test]
    fn real_force_only_after_graceful_timeout_and_second_confirmation() {
        let mut service = service(true);
        let core = Core::default();
        let plan = core.prepare_close(service.child.id()).unwrap();
        assert!(core.force_close(&plan.token).is_err());
        assert!(service.child.try_wait().unwrap().is_none());
        let plan = core.prepare_close(service.child.id()).unwrap();
        let start = Instant::now();
        let outcome = core.execute_close(&plan.token).unwrap();
        assert!(start.elapsed() >= CLOSE_WAIT);
        assert_eq!(outcome.status, "stillListening");
        assert_eq!(outcome.remaining_ports.len(), 2);
        assert!(service.child.try_wait().unwrap().is_none());
        let token = outcome.force_token.unwrap();
        let outcome = core.force_close(&token).unwrap();
        assert_eq!(outcome.status, "closed");
        assert!(core.force_close(&token).is_err());
        service.child.wait().unwrap();
    }

    #[test]
    fn real_port_change_invalidates_confirmation_without_sending_signal() {
        let mut service = service(false);
        let core = Core::default();
        let plan = core.prepare_close(service.child.id()).unwrap();
        fs::write(service.root.join("add-port"), "").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !service.root.join("added").exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(20));
        }
        let result = core.execute_close(&plan.token).unwrap_err();
        assert!(result.contains("监听端口或绑定地址已变化"));
        assert!(service.child.try_wait().unwrap().is_none());
    }

    #[test]
    fn nearest_marker_keeps_nested_project_and_worktree_separate() {
        let service = service(false);
        fs::create_dir(service.root.join(".git")).unwrap();
        let nested = service.root.join("packages/web");
        fs::create_dir_all(nested.join("src")).unwrap();
        fs::write(nested.join("package.json"), "{}").unwrap();
        assert_eq!(
            project_for(nested.join("src").to_str().unwrap())
                .unwrap()
                .root,
            nested.to_str().unwrap()
        );
        let worktree = service.root.join("another-worktree");
        fs::create_dir(&worktree).unwrap();
        fs::write(worktree.join(".git"), "gitdir: /tmp/example-worktree").unwrap();
        assert_eq!(
            project_for(worktree.to_str().unwrap()).unwrap().root,
            worktree.to_str().unwrap()
        );
    }
}
