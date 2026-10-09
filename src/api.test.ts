import { beforeEach, describe, expect, it, vi } from 'vitest';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { watchPanelActivity } from './api';

vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

let focusHandler: (event: { payload: boolean }) => void;
const native = { isVisible: vi.fn(), isFocused: vi.fn(), onFocusChanged: vi.fn() };
const unlisten = vi.fn();
beforeEach(() => {
  vi.resetAllMocks();
  native.isVisible.mockResolvedValue(true);
  native.isFocused.mockResolvedValue(true);
  native.onFocusChanged.mockImplementation(async handler => { focusHandler = handler; return unlisten; });
  vi.mocked(getCurrentWindow).mockReturnValue(native as unknown as ReturnType<typeof getCurrentWindow>);
});

describe('原生窗口状态', () => {
  it('监听先建立，再读取原生可见与焦点状态，不依赖 DOM focus', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    const update = vi.fn();
    const stop = await watchPanelActivity(update, vi.fn());
    expect(update).toHaveBeenCalledExactlyOnceWith(true);
    expect(native.onFocusChanged.mock.invocationCallOrder[0]).toBeLessThan(native.isVisible.mock.invocationCallOrder[0]);
    expect(native.onFocusChanged.mock.invocationCallOrder[0]).toBeLessThan(native.isFocused.mock.invocationCallOrder[0]);
    focusHandler({ payload: false });
    expect(update).toHaveBeenLastCalledWith(false);
    stop();
    expect(unlisten).toHaveBeenCalledTimes(1);
    focusHandler({ payload: true });
    await Promise.resolve(); await Promise.resolve();
    expect(update).toHaveBeenCalledTimes(2);
  });

  it('隐藏后台窗口即使焦点标志为真也不启动轮询', async () => {
    native.isVisible.mockResolvedValue(false);
    const update = vi.fn();
    const stop = await watchPanelActivity(update, vi.fn());
    expect(update).toHaveBeenCalledExactlyOnceWith(false);
    stop();
  });

  it('初始状态读取途中收到失焦，不让过期读取恢复轮询', async () => {
    let resolveVisible!: (value: boolean) => void;
    native.isVisible.mockImplementationOnce(() => new Promise(resolve => { resolveVisible = resolve; }));
    const update = vi.fn();
    const stopping = watchPanelActivity(update, vi.fn());
    await Promise.resolve();
    focusHandler({ payload: false });
    resolveVisible(true);
    const stop = await stopping;
    expect(update).toHaveBeenCalledExactlyOnceWith(false);
    stop();
  });

  it('初次原生读取失败释放监听，不把错误当成可见', async () => {
    native.isVisible.mockRejectedValueOnce(new Error('权限拒绝'));
    const update = vi.fn();
    await expect(watchPanelActivity(update, vi.fn())).rejects.toThrow('权限拒绝');
    expect(update).not.toHaveBeenCalled();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });
});
