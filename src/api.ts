import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { CloseOutcome, ClosePlan, Preferences, Snapshot } from './types';

export function desktopAvailable(): boolean {
  return '__TAURI_INTERNALS__' in window;
}

function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktopAvailable()) return Promise.reject(new Error('桌面功能不可用。请在 PortNest 桌面应用中打开此页面。'));
  return invoke<T>(name, args);
}

export function isSettingsWindow(): boolean {
  const query = new URLSearchParams(window.location.search);
  return query.get('view') === 'settings' || query.get('window') === 'settings'
    || (desktopAvailable() && getCurrentWindow().label === 'settings');
}

export const api = {
  scan: () => command<Snapshot>('scan_ports'),
  prepare: (pid: number) => command<ClosePlan>('prepare_close', { pid }),
  close: (token: string) => command<CloseOutcome>('execute_close', { token }),
  force: (token: string) => command<CloseOutcome>('force_close', { token }),
  open: (pid: number, port: number) => command<void>('open_port', { pid, port }),
  settings: () => command<void>('show_settings'),
  preferences: () => command<Preferences>('get_preferences'),
  autostart: (enabled: boolean) => command<Preferences>('set_autostart', { enabled }),
  hide: () => command<void>('hide_panel'),
  quit: () => command<void>('quit_app'),
};

// WKWebView DOM focus can lag behind the native window's first display.
export async function watchPanelActivity(onChange: (active: boolean) => void, onError: (error: unknown) => void): Promise<() => void> {
  const panel = getCurrentWindow();
  let listening = true;
  let revision = 0;
  const refresh = async (focused?: boolean) => {
    const current = ++revision;
    const [visible, hasFocus] = await Promise.all([panel.isVisible(), focused === undefined ? panel.isFocused() : Promise.resolve(focused)]);
    if (listening && current === revision) onChange(visible && hasFocus);
  };
  const unlisten = await panel.onFocusChanged(({ payload }) => {
    if (!listening) return;
    if (!payload) { revision++; if (listening) onChange(false); }
    else void refresh(true).catch(error => { if (listening) { onChange(false); onError(error); } });
  });
  try { await refresh(); }
  catch (error) { listening = false; unlisten(); throw error; }
  return () => { listening = false; revision++; unlisten(); };
}
