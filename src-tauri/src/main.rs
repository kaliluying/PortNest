#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use portnest_core::{CloseOutcome, ClosePlan, Core, Snapshot};
use serde::Serialize;
use std::{net::IpAddr, process::Command, sync::Arc};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    window::{Effect, EffectState, EffectsBuilder},
    AppHandle, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;

type Engine = Arc<Core>;

#[tauri::command]
async fn scan_ports(core: State<'_, Engine>) -> Result<Snapshot, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.scan())
        .await
        .map_err(|_| "端口扫描任务异常，请重试。".to_string())?
}

#[tauri::command]
async fn prepare_close(pid: u32, core: State<'_, Engine>) -> Result<ClosePlan, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.prepare_close(pid))
        .await
        .map_err(|_| "无法准备关闭操作，请刷新后重试。".to_string())?
}

#[tauri::command]
async fn execute_close(token: String, core: State<'_, Engine>) -> Result<CloseOutcome, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.execute_close(&token))
        .await
        .map_err(|_| "关闭任务异常，请刷新查看进程当前状态。".to_string())?
}

#[tauri::command]
async fn force_close(token: String, core: State<'_, Engine>) -> Result<CloseOutcome, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.force_close(&token))
        .await
        .map_err(|_| "强制关闭任务异常，请刷新查看进程当前状态。".to_string())?
}

fn browser_url(addresses: &[String], port: u16) -> Result<String, String> {
    let mut ips = addresses.iter().filter_map(|address| {
        if address == "*" {
            return Some(IpAddr::from([127, 0, 0, 1]));
        }
        address.parse::<IpAddr>().ok()
    });
    let ip = ips
        .find(|ip| ip.is_loopback() || ip.is_unspecified())
        .or_else(|| addresses.iter().find_map(|address| address.parse().ok()))
        .ok_or("无法确认此端口的本机地址，请查看监听详情。")?;
    let host = match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => "127.0.0.1".to_string(),
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) if ip.is_unspecified() => "[::1]".to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    };
    Ok(format!("http://{host}:{port}"))
}

#[tauri::command]
async fn open_port(pid: u32, port: u16, core: State<'_, Engine>) -> Result<(), String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = core.scan()?;
        let listener = snapshot
            .processes
            .iter()
            .find(|process| process.pid == pid)
            .and_then(|process| {
                process
                    .listeners
                    .iter()
                    .find(|listener| listener.port == port)
            })
            .ok_or("该进程已不再监听这个端口，请刷新列表。")?;
        let url = browser_url(&listener.addresses, port)?;
        let status = Command::new("/usr/bin/open")
            .arg(url)
            .status()
            .map_err(|_| "无法打开默认浏览器，请检查系统设置。".to_string())?;
        if !status.success() {
            return Err("默认浏览器未能打开此地址，请检查系统设置。".to_string());
        }
        Ok(())
    })
    .await
    .map_err(|_| "浏览器打开任务异常，请重试。".to_string())?
}

#[derive(Serialize)]
struct Preferences {
    autostart: bool,
}

#[tauri::command]
fn get_preferences(app: AppHandle) -> Result<Preferences, String> {
    let autostart = app
        .autolaunch()
        .is_enabled()
        .map_err(|_| "无法读取登录启动状态，请重试。".to_string())?;
    Ok(Preferences { autostart })
}

#[tauri::command]
fn set_autostart(enabled: bool, app: AppHandle) -> Result<Preferences, String> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|_| "登录启动设置未能保存，请检查系统权限后重试。".to_string())?;
    get_preferences(app)
}

#[tauri::command]
fn show_settings(app: AppHandle) -> Result<(), String> {
    let window = if let Some(window) = app.get_webview_window("settings") {
        window
    } else {
        WebviewWindowBuilder::new(
            &app,
            "settings",
            WebviewUrl::App("index.html?view=settings".into()),
        )
        .title("PortNest 设置")
        .inner_size(460.0, 520.0)
        .resizable(false)
        .center()
        .build()
        .map_err(|_| "设置窗口未能打开，请重试。".to_string())?
    };
    hide_panel(app.clone())?;
    window
        .show()
        .map_err(|_| "设置窗口未能显示。".to_string())?;
    window
        .set_focus()
        .map_err(|_| "设置窗口未能获得焦点。".to_string())
}

#[tauri::command]
fn hide_panel(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("panel") {
        window
            .hide()
            .map_err(|_| "面板未能隐藏，请重试。".to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

fn show_panel(app: &AppHandle, rect: Option<tauri::Rect>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("panel") else {
        return Ok(());
    };
    let rect = rect.or_else(|| {
        app.tray_by_id("portnest")
            .and_then(|tray| tray.rect().ok().flatten())
    });
    if let Some(rect) = rect {
        let scale = window.scale_factor()?;
        let position = rect.position.to_physical::<f64>(scale);
        let size = rect.size.to_physical::<f64>(scale);
        let monitors = window.available_monitors()?;
        let monitor = monitors.iter().find(|monitor| {
            let origin = monitor.position();
            let extent = monitor.size();
            position.x >= origin.x as f64
                && position.x < origin.x as f64 + extent.width as f64
                && position.y >= origin.y as f64
                && position.y < origin.y as f64 + extent.height as f64
        });
        let outer = window.outer_size()?;
        let mut x = position.x + size.width / 2.0 - outer.width as f64 / 2.0;
        let mut y = position.y + size.height + 6.0 * scale;
        if let Some(monitor) = monitor {
            let area = monitor.work_area();
            let margin = 8.0 * monitor.scale_factor();
            let min_x = area.position.x as f64 + margin;
            let max_x =
                (area.position.x as f64 + area.size.width as f64 - outer.width as f64 - margin)
                    .max(min_x);
            let min_y = area.position.y as f64 + margin;
            let max_y =
                (area.position.y as f64 + area.size.height as f64 - outer.height as f64 - margin)
                    .max(min_y);
            x = x.clamp(min_x, max_x);
            y = y.clamp(min_y, max_y);
        }
        window.set_position(PhysicalPosition::new(x as i32, y as i32))?;
    } else {
        window.center()?;
    }
    window.show()?;
    window.set_focus()
}

fn main() {
    tauri::Builder::default()
        .manage(Arc::new(Core::default()))
        .invoke_handler(tauri::generate_handler![
            scan_ports,
            prepare_close,
            execute_close,
            force_close,
            open_port,
            get_preferences,
            set_autostart,
            show_settings,
            hide_panel,
            quit_app
        ])
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let _ = show_panel(app, None);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let show = MenuItem::with_id(app, "show", "打开 PortNest", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出 PortNest", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &settings, &quit])?;
            TrayIconBuilder::with_id("portnest")
                .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .icon_as_template(true)
                .tooltip("PortNest · 本地开发端口")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        let _ = show_panel(app, None);
                    }
                    "settings" => {
                        let _ = show_settings(app.clone());
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if app
                            .get_webview_window("panel")
                            .is_some_and(|window| window.is_visible().unwrap_or(false))
                        {
                            let _ = hide_panel(app.clone());
                        } else {
                            let _ = show_panel(app, Some(rect));
                        }
                    }
                })
                .build(app)?;
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("panel") {
                window.set_effects(
                    EffectsBuilder::new()
                        .effect(Effect::Popover)
                        .state(EffectState::Active)
                        .radius(18.0)
                        .build(),
                )?;
            }
            if !std::env::args().any(|argument| argument == "--background") {
                show_panel(app.handle(), None)?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "panel" {
                match event {
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    WindowEvent::Focused(false) => {
                        let _ = window.hide();
                    }
                    _ => {}
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("PortNest 未能启动");
}

#[cfg(test)]
mod tests {
    use super::browser_url;

    #[test]
    fn browser_addresses_only_use_verified_numeric_bindings() {
        assert_eq!(
            browser_url(&["*".into()], 3000).unwrap(),
            "http://127.0.0.1:3000"
        );
        assert_eq!(
            browser_url(&["::".into()], 5173).unwrap(),
            "http://[::1]:5173"
        );
        assert_eq!(
            browser_url(&["192.168.1.12".into()], 8080).unwrap(),
            "http://192.168.1.12:8080"
        );
        assert!(browser_url(&["example.com".into()], 3000).is_err());
        assert!(browser_url(&["127.0.0.1;touch /tmp/x".into()], 3000).is_err());
    }
}
