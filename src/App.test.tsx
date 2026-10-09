import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { api, desktopAvailable, isSettingsWindow, watchPanelActivity } from './api';
import type { ProcessRecord, Snapshot } from './types';

vi.mock('./api', () => ({
  isSettingsWindow: vi.fn(() => false), desktopAvailable: vi.fn(() => false), watchPanelActivity: vi.fn(),
  api: { scan: vi.fn(), prepare: vi.fn(), close: vi.fn(), force: vi.fn(), open: vi.fn(), settings: vi.fn(), preferences: vi.fn(), autostart: vi.fn(), hide: vi.fn(), quit: vi.fn() },
}));

const service: ProcessRecord = {
  id: '900-identity', pid: 900, uid: 501, name: 'node', executable: '/usr/local/bin/node', cwd: '/projects/nest', startedAt: 1_700_000_000_000,
  project: { name: 'nest', root: '/projects/nest', marker: 'package.json' }, category: 'development', protectionReason: null, canClose: true,
  listeners: [{ port: 3000, addresses: ['127.0.0.1'], scope: 'local' }, { port: 3001, addresses: ['::1'], scope: 'local' }],
};
const snapshot = (processes = [service]): Snapshot => ({ processes, scannedAt: 1_700_000_000_000, limitations: [] });

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(isSettingsWindow).mockReturnValue(false);
  vi.mocked(desktopAvailable).mockReturnValue(false);
  vi.mocked(api.scan).mockResolvedValue(snapshot());
  vi.mocked(api.prepare).mockResolvedValue({ token: 'server-plan', process: service, expiresAt: Date.now() + 30_000 });
  vi.mocked(api.close).mockResolvedValue({ status: 'closed', pid: 900, remainingPorts: [], message: '全部端口已释放', forceToken: null });
  vi.mocked(api.open).mockResolvedValue();
  vi.spyOn(document, 'hasFocus').mockReturnValue(true);
});
afterEach(() => { cleanup(); vi.useRealTimers(); });

async function renderPanel() { render(<App />); await screen.findByText('nest'); }

describe('端口面板', () => {
  it('按端口查找进程，详情显示全部端口与绑定地址，仅点击打开才请求浏览器', async () => {
    await renderPanel();
    expect(api.open).not.toHaveBeenCalled();
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: '3001' } });
    expect(screen.getByText('nest')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '查看 node 详情' }));
    expect(screen.getAllByText('/projects/nest').length).toBe(2);
    expect(screen.getByText('::1')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '在浏览器打开端口 3001' }));
    expect(api.open).toHaveBeenCalledExactlyOnceWith(900, 3001);
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: '9876' } });
    expect(screen.getByText('没有匹配的进程')).toBeTruthy();
  });

  it('确认展示 prepare 返回的最新全部端口，仅提交服务器 token，取消不关闭', async () => {
    const latest = { ...service, listeners: [...service.listeners, { port: 4000, addresses: ['*'], scope: 'lan' as const }] };
    vi.mocked(api.prepare).mockResolvedValue({ token: 'latest-plan', process: latest, expiresAt: Date.now() + 30_000 });
    await renderPanel();
    fireEvent.click(screen.getByRole('button', { name: '关闭 node' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('4000')).toBeTruthy();
    expect(api.close).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole('button', { name: '取消' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(api.close).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '关闭 node' }));
    fireEvent.click(await screen.findByRole('button', { name: '正常关闭' }));
    await screen.findByText('进程已关闭');
    expect(api.close).toHaveBeenCalledExactlyOnceWith('latest-plan');
    expect(api.force).not.toHaveBeenCalled();
  });

  it('正常关闭失败后，强制关闭必须再次确认，传递独立 force token', async () => {
    vi.mocked(api.close).mockResolvedValue({ status: 'stillListening', pid: 900, remainingPorts: [3000, 3001], message: '等待 5 秒后仍在监听', forceToken: 'force-only' });
    vi.mocked(api.force).mockResolvedValue({ status: 'closed', pid: 900, remainingPorts: [], message: '强制结束后端口已释放', forceToken: null });
    await renderPanel();
    fireEvent.click(screen.getByRole('button', { name: '关闭 node' }));
    fireEvent.click(await screen.findByRole('button', { name: '正常关闭' }));
    fireEvent.click(await screen.findByRole('button', { name: '确认强制关闭' }));
    expect(api.force).not.toHaveBeenCalled();
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByText('3000')).toBeTruthy();
    expect(within(dialog).getByText('3001')).toBeTruthy();
    fireEvent.click(within(dialog).getByRole('button', { name: '强制关闭进程' }));
    await screen.findByText('进程已关闭');
    expect(api.force).toHaveBeenCalledExactlyOnceWith('force-only');
  });

  it('受保护项默认折叠且不能关闭，未知归属须先看详情', async () => {
    const protectedProcess: ProcessRecord = { ...service, id: '1-protected', pid: 1, name: 'launchd', project: null, category: 'protected', canClose: false, protectionReason: '系统进程', listeners: [{ port: 9000, addresses: ['*'], scope: 'lan' }] };
    const unknown: ProcessRecord = { ...service, project: null, category: 'unknown' };
    vi.mocked(api.scan).mockResolvedValue(snapshot([unknown, protectedProcess]));
    render(<App />);
    await screen.findByText('node');
    expect(screen.queryByText('launchd')).toBeNull();
    expect(screen.queryByRole('button', { name: '关闭 node' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /归属未知，查看详情后可关闭/ }));
    expect(screen.getByRole('button', { name: '关闭 node' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /受保护进程/ }));
    expect(screen.getByText('launchd')).toBeTruthy();
    expect(screen.queryByRole('button', { name: '关闭 launchd' })).toBeNull();
  });

  it('扫描失败保留之前结果，显示结果时间而不是空列表，重试恢复', async () => {
    await renderPanel();
    vi.mocked(api.scan).mockRejectedValueOnce(new Error('读取超时'));
    fireEvent.click(screen.getByRole('button', { name: '刷新端口' }));
    await screen.findByText('刷新失败，正在显示上次结果');
    expect(screen.getByText('nest')).toBeTruthy();
    expect(screen.getByText(/结果时间：/)).toBeTruthy();
    expect(screen.queryByText('没有监听服务')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  });

  it('空列表与初次扫描错误分别显示', async () => {
    vi.mocked(api.scan).mockResolvedValueOnce(snapshot([]));
    render(<App />);
    await screen.findByText('没有监听服务');
    cleanup();
    vi.mocked(api.scan).mockRejectedValueOnce(new Error('桌面功能不可用'));
    render(<App />);
    await screen.findByText('无法读取监听端口');
    expect(screen.queryByText('没有监听服务')).toBeNull();
  });

  it('失焦停止轮询，焦点恢复立即刷新，扫描保持互斥', async () => {
    vi.useFakeTimers();
    render(<App />);
    await vi.advanceTimersByTimeAsync(0);
    expect(api.scan).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(api.scan).toHaveBeenCalledTimes(2);
    vi.mocked(document.hasFocus).mockReturnValue(false);
    fireEvent(window, new Event('blur'));
    await vi.advanceTimersByTimeAsync(15_000);
    expect(api.scan).toHaveBeenCalledTimes(2);
    vi.mocked(document.hasFocus).mockReturnValue(true);
    fireEvent(window, new Event('focus'));
    await vi.advanceTimersByTimeAsync(0);
    expect(api.scan).toHaveBeenCalledTimes(3);
    let resolveScan!: (value: Snapshot) => void;
    vi.mocked(api.scan).mockImplementationOnce(() => new Promise(resolve => { resolveScan = resolve; }));
    await vi.advanceTimersByTimeAsync(5000);
    await vi.advanceTimersByTimeAsync(15_000);
    expect(api.scan).toHaveBeenCalledTimes(4);
    resolveScan(snapshot());
    await vi.advanceTimersByTimeAsync(0);
  });

  it('原生面板初次隐藏时不扫描，打开取得焦点后才扫描', async () => {
    vi.mocked(document.hasFocus).mockReturnValue(false);
    vi.mocked(desktopAvailable).mockReturnValue(true);
    let activity!: (active: boolean) => void;
    vi.mocked(watchPanelActivity).mockImplementation(async callback => { activity = callback; callback(false); return () => {}; });
    render(<App />);
    expect(api.scan).not.toHaveBeenCalled();
    activity(true);
    await screen.findByText('nest');
    expect(api.scan).toHaveBeenCalledTimes(1);
    expect(document.hasFocus()).toBe(false);
  });

  it('原生窗口显示时扫描，隐藏按钮立即停止原生轮询', async () => {
    vi.useFakeTimers();
    vi.mocked(desktopAvailable).mockReturnValue(true);
    vi.mocked(document.hasFocus).mockReturnValue(false);
    const unlisten = vi.fn();
    vi.mocked(watchPanelActivity).mockImplementation(async callback => { callback(true); return unlisten; });
    vi.mocked(api.hide).mockResolvedValue();
    render(<App />);
    await vi.advanceTimersByTimeAsync(0);
    expect(api.scan).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(api.scan).toHaveBeenCalledTimes(2);
    fireEvent.click(screen.getByRole('button', { name: '隐藏面板' }));
    await vi.advanceTimersByTimeAsync(15_000);
    expect(api.hide).toHaveBeenCalledTimes(1);
    expect(api.scan).toHaveBeenCalledTimes(2);
    cleanup();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it('搜索计数仅包含筛选进程的端口', async () => {
    vi.mocked(api.scan).mockResolvedValue(snapshot([service, { ...service, id: 'other', pid: 901, name: 'python', cwd: '/projects/other', project: { name: 'other', root: '/projects/other', marker: 'pyproject.toml' }, listeners: [{ port: 8888, addresses: ['127.0.0.1'], scope: 'local' }] }]));
    await renderPanel();
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'nest' } });
    expect(screen.getByText(/2 个 TCP 端口/)).toBeTruthy();
    expect(screen.queryByText('other')).toBeNull();
  });

  it('关闭错误不报告成功，对话框支持 Escape、焦点恢复和 Tab 限制', async () => {
    vi.mocked(api.close).mockRejectedValue(new Error('身份变化，已中止'));
    await renderPanel();
    const trigger = screen.getByRole('button', { name: '关闭 node' });
    trigger.focus(); fireEvent.click(trigger);
    let dialog = await screen.findByRole('dialog');
    const cancel = within(dialog).getByRole('button', { name: '取消' });
    expect(document.activeElement).toBe(cancel);
    fireEvent.keyDown(cancel, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(within(dialog).getByRole('button', { name: '正常关闭' }));
    fireEvent.keyDown(dialog, { key: 'Escape' });
    expect(document.activeElement).toBe(trigger);
    fireEvent.click(trigger);
    dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: '正常关闭' }));
    await screen.findByText('身份变化，已中止');
    expect(screen.queryByText('进程已关闭')).toBeNull();
  });
});

describe('设置', () => {
  it('读取真实偏好，保存失败保留原开关状态并显示错误', async () => {
    vi.mocked(isSettingsWindow).mockReturnValue(true);
    vi.mocked(api.preferences).mockResolvedValue({ autostart: true });
    vi.mocked(api.autostart).mockRejectedValue(new Error('登录项更新失败'));
    render(<App />);
    const toggle = screen.getByRole('switch');
    await waitFor(() => expect(toggle.getAttribute('aria-checked')).toBe('true'));
    fireEvent.click(toggle);
    await screen.findByText('登录项更新失败');
    expect(toggle.getAttribute('aria-checked')).toBe('true');
    expect(api.autostart).toHaveBeenCalledExactlyOnceWith(false);
    expect(api.scan).not.toHaveBeenCalled();
  });
  it('初次系统偏好读取失败时禁止基于猜测切换登录项', async () => {
    vi.mocked(isSettingsWindow).mockReturnValue(true);
    vi.mocked(api.preferences).mockRejectedValue(new Error('无法读取登录项'));
    render(<App />);
    await screen.findByText('无法读取登录项');
    expect((screen.getByRole('switch') as HTMLButtonElement).disabled).toBe(true);
    expect(api.autostart).not.toHaveBeenCalled();
  });
});
