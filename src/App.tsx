import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { ChevronDown, ChevronRight, ExternalLink, LoaderCircle, Power, RefreshCw, Search, Settings, ShieldCheck, X } from 'lucide-react';
import { api, desktopAvailable, isSettingsWindow, watchPanelActivity } from './api';
import type { CloseOutcome, ClosePlan, ProcessRecord, Snapshot } from './types';

const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
const formatTime = (time: number) => new Date(time).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
const formatStart = (time: number | null) => time === null ? '无法读取' : new Date(time).toLocaleString('zh-CN');

function BrandMark() {
  return <svg className="brand-mark" viewBox="0 0 1024 1024" aria-hidden="true">
    <g fill="none" stroke="#fff" strokeWidth="60" strokeLinecap="round" strokeLinejoin="round">
      <path d="M272 398v132c0 153 105 250 240 250s240-97 240-250V398" />
      <path d="M374 454v90c0 95 59 151 138 151s138-56 138-151v-90" />
      <path d="M476 474v68c0 32 14 51 36 51s36-19 36-51v-68" />
    </g><circle cx="512" cy="300" r="63" fill="#fff" />
  </svg>;
}

function Dialog({ title, children, busy, onCancel, onConfirm, confirmLabel, danger = false }: {
  title: string; children: ReactNode; busy: boolean; onCancel: () => void;
  onConfirm: () => void; confirmLabel: string; danger?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (busy) ref.current?.focus(); }, [busy]);
  const cancel = useRef(onCancel);
  cancel.current = onCancel;
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    cancelRef.current?.focus();
    return () => { if (previous?.isConnected) previous.focus(); };
  }, []);
  return <div className="dialog-backdrop">
    <div role="dialog" aria-modal="true" aria-labelledby="dialog-title" aria-describedby="dialog-description" className="dialog" ref={ref} tabIndex={-1} aria-busy={busy}
      onKeyDown={event => {
        if (event.key === 'Escape' && !busy) { event.preventDefault(); cancel.current(); }
        if (event.key !== 'Tab') return;
        const items = [...(ref.current?.querySelectorAll<HTMLElement>('button:not(:disabled), [tabindex="0"]') ?? [])];
        const first = items[0]; const last = items.at(-1);
        if (!first) { event.preventDefault(); return; }
        if (!items.includes(document.activeElement as HTMLElement)) { event.preventDefault(); first.focus(); return; }
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
      }}>
      <span className={`dialog-symbol ${danger ? 'danger' : ''}`}><Power size={22} /></span>
      <h2 id="dialog-title">{title}</h2>
      <div id="dialog-description">{children}</div>
      <div className="dialog-actions">
        <button ref={cancelRef} className="secondary" onClick={onCancel} disabled={busy}>取消</button>
        <button className={danger ? 'danger-button' : 'primary'} onClick={onConfirm} disabled={busy}>
          {busy && <LoaderCircle className="spin" size={15} />}{busy ? '正在关闭…' : confirmLabel}
        </button>
      </div>
      {busy && <p className="muted waiting" role="status">正在核验进程并等待端口释放，最多 5 秒。</p>}
    </div>
  </div>;
}

function ProcessRow({ process, busy, onClose, onOpen }: {
  process: ProcessRecord; busy: boolean; onClose: (process: ProcessRecord) => void; onOpen: (pid: number, port: number) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const canOfferClose = process.canClose && (process.category !== 'unknown' || expanded);
  return <article className={`process-card ${process.category === 'protected' ? 'protected' : ''}`}>
    <div className="process-summary">
      <button className="process-disclosure" aria-expanded={expanded} aria-label={`${expanded ? '收起' : '查看'} ${process.name} 详情`} onClick={() => setExpanded(value => !value)}>
        <span className="process-icon">{process.category === 'protected' ? <ShieldCheck size={18} /> : <span className="project-initial">{(process.project?.name ?? process.name).slice(0, 1).toUpperCase()}</span>}</span>
        <span className="process-heading"><strong>{process.project?.name ?? process.name}</strong><span>{process.project ? process.name : '未知归属'}<span className="dot">·</span>PID {process.pid}</span></span>
        {expanded ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
      </button>
      {canOfferClose && <button className="close-button" disabled={busy} onClick={() => onClose(process)} aria-label={`关闭 ${process.name}`}><Power size={14} /><span>关闭</span></button>}
    </div>
    <div className="port-list">{process.listeners.map(listener => <span className={`port-badge ${listener.scope === 'lan' ? 'lan' : ''}`} key={listener.port} title={`${listener.scope === 'local' ? '仅本机' : '可能对局域网开放，未验证可达性'}：${listener.addresses.join(', ')}`}><span className="status-dot" /><span>{listener.port}</span><span className="scope-short">{listener.scope === 'local' ? '本机' : '可能局域网'}</span></span>)}</div>
    {process.category === 'unknown' && !expanded && process.canClose && <button className="detail-hint" onClick={() => setExpanded(true)}>归属未知，查看详情后可关闭 <ChevronRight size={12} /></button>}
    {expanded && <div className="process-details">
      <dl><dt>项目目录</dt><dd>{process.project?.root ?? '未知归属'}</dd>{process.project && <><dt>识别依据</dt><dd>{process.project.marker}</dd></>}
        <dt>工作目录</dt><dd>{process.cwd ?? '无法读取'}</dd><dt>可执行文件</dt><dd>{process.executable ?? '无法读取'}</dd>
        <dt>进程 / 用户</dt><dd>PID {process.pid} / UID {process.uid ?? '未知'}</dd><dt>启动时间</dt><dd>{formatStart(process.startedAt)}</dd>
      </dl>
      <div className="listener-details">{process.listeners.map(listener => <div key={listener.port}><div className="listener-title"><strong>{listener.port}</strong><span>{listener.scope === 'local' ? '仅本机' : '可能对局域网开放'}</span><button className="text-button" disabled={busy} onClick={() => onOpen(process.pid, listener.port)} aria-label={`在浏览器打开端口 ${listener.port}`}><ExternalLink size={13} />打开</button></div><code>{listener.addresses.join(' · ')}</code></div>)}</div>
      {process.listeners.some(listener => listener.scope === 'lan') && <p className="muted">绑定地址包含回环以外地址；未验证其他设备能否访问。</p>}
      {!process.canClose && <p className="protection-note"><ShieldCheck size={14} />{process.protectionReason ?? '无法充分核验身份，已保护此进程。'}</p>}
      {process.category === 'unknown' && process.canClose && <p className="muted">无法确认项目归属，请核对目录、进程与端口再操作。</p>}
    </div>}
  </article>;
}

function SettingsView() {
  const [autostart, setAutostart] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [preferencesReady, setPreferencesReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(false);
  const readPreferences = useCallback(async () => {
    setLoading(true); setError(null);
    try { const prefs = await api.preferences(); if (mounted.current) { setAutostart(prefs.autostart); setPreferencesReady(true); } }
    catch (cause) { if (mounted.current) setError(errorText(cause)); }
    finally { if (mounted.current) setLoading(false); }
  }, []);
  useEffect(() => { mounted.current = true; void readPreferences(); return () => { mounted.current = false; }; }, [readPreferences]);
  async function changeAutostart() {
    setSaving(true); setError(null);
    try { const prefs = await api.autostart(!autostart); if (mounted.current) setAutostart(prefs.autostart); }
    catch (cause) { if (mounted.current) setError(errorText(cause)); }
    finally { if (mounted.current) setSaving(false); }
  }
  return <main className="settings-page"><div className="settings-brand"><BrandMark /><div><h1>PortNest</h1><p>设置</p></div></div>
    <section className="settings-group"><div className="preference-row"><div><strong>登录时启动</strong><p>登录 Mac 后，PortNest 显示在菜单栏。</p></div><button type="button" role="switch" aria-checked={autostart} aria-label="登录时启动" className="switch" disabled={loading || saving || !preferencesReady} onClick={() => void changeAutostart()}><span /></button></div>
      <div className="preference-row"><div><strong>外观</strong><p>跟随系统的浅色与深色主题。</p></div><span className="subtle-label">系统</span></div>
      <div className="preference-row"><div><strong>语言</strong><p>首版使用中文界面。</p></div><span className="subtle-label">简体中文</span></div>
    </section>
    {loading && <p className="muted" role="status">正在读取系统设置…</p>}{saving && <p className="muted" role="status">正在更新登录启动…</p>}
    {error && <div className="error-banner" role="alert"><span>{error}</span><button className="text-button" onClick={() => void readPreferences()} disabled={loading || saving}>重试</button></div>}
    <p className="settings-footnote">本机处理，不主动探测端口、不发送遥测。<br />关闭进程前会重新核验身份与影响范围。</p>
    <button className="secondary quit-button" onClick={() => { void api.quit().catch(cause => setError(errorText(cause))); }}><Power size={15} />退出 PortNest</button>
  </main>;
}

function Panel() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [showProtected, setShowProtected] = useState(false);
  const [plan, setPlan] = useState<ClosePlan | null>(null);
  const [outcome, setOutcome] = useState<{ result: CloseOutcome; process: ProcessRecord } | null>(null);
  const [forceConfirm, setForceConfirm] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [closing, setClosing] = useState(false);
  const scanPending = useRef(false);
  const setPanelActive = useRef<(active: boolean) => void>(() => {});
  const mounted = useRef(false);
  const operationPending = useRef(false);
  const dialogOpen = useRef(false);
  dialogOpen.current = plan !== null || forceConfirm;
  const scan = useCallback(async () => {
    if (scanPending.current || operationPending.current || dialogOpen.current) return;
    scanPending.current = true; setLoading(true);
    try { const result = await api.scan(); if (mounted.current) { setSnapshot(result); setError(null); } }
    catch (cause) { if (mounted.current) setError(errorText(cause)); }
    finally { scanPending.current = false; if (mounted.current) setLoading(false); }
  }, []);
  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let timer: ReturnType<typeof setInterval> | undefined;
    let unlistenNative: (() => void) | undefined;
    const setActive = (active: boolean) => {
      clearInterval(timer);
      if (disposed) return;
      if (active) { void scan(); timer = setInterval(() => void scan(), 5000); }
    };
    setPanelActive.current = setActive;
    const updateBrowserPolling = () => setActive(document.visibilityState === 'visible' && document.hasFocus());
    const onEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !dialogOpen.current && !operationPending.current) hidePanel();
    };
    if (desktopAvailable()) {
      void watchPanelActivity(setActive, cause => { if (!disposed) setActionError(errorText(cause)); })
        .then(unlisten => { if (disposed) unlisten(); else unlistenNative = unlisten; })
        .catch(cause => { if (!disposed) setActionError(errorText(cause)); });
    } else {
      void scan();
      updateBrowserPolling();
      window.addEventListener('focus', updateBrowserPolling); window.addEventListener('blur', updateBrowserPolling);
      document.addEventListener('visibilitychange', updateBrowserPolling);
    }
    window.addEventListener('keydown', onEscape);
    return () => {
      disposed = true; mounted.current = false; clearInterval(timer); unlistenNative?.(); setPanelActive.current = () => {};
      window.removeEventListener('focus', updateBrowserPolling); window.removeEventListener('blur', updateBrowserPolling);
      document.removeEventListener('visibilitychange', updateBrowserPolling); window.removeEventListener('keydown', onEscape);
    };
  }, [scan]);
  function hidePanel() {
    setPanelActive.current(false);
    report(api.hide());
  }
  async function prepare(process: ProcessRecord) {
    if (operationPending.current) return;
    operationPending.current = true; setPreparing(true); setActionError(null); setOutcome(null);
    try { const nextPlan = await api.prepare(process.pid); if (mounted.current) setPlan(nextPlan); }
    catch (cause) { if (mounted.current) setActionError(errorText(cause)); }
    finally { operationPending.current = false; if (mounted.current) setPreparing(false); }
  }
  async function execute(force: boolean) {
    const token = force ? outcome?.result.forceToken : plan?.token;
    const process = force ? outcome?.process : plan?.process;
    if (!token || !process || operationPending.current) return;
    operationPending.current = true; setClosing(true); setActionError(null);
    try {
      const result = await (force ? api.force(token) : api.close(token));
      if (mounted.current) { setPlan(null); setForceConfirm(false); setOutcome({ result, process }); }
    } catch (cause) {
      if (mounted.current) { setActionError(errorText(cause)); setPlan(null); setForceConfirm(false); setOutcome(null); }
    } finally {
      operationPending.current = false;
      if (mounted.current) { setClosing(false); dialogOpen.current = false; void scan(); }
    }
  }
  function report(action: Promise<void>) { void action.catch(cause => { if (mounted.current) setActionError(errorText(cause)); }); }
  const processes = snapshot?.processes ?? [];
  const normalized = query.trim().toLowerCase();
  const matches = (process: ProcessRecord) => !normalized || [process.name, process.project?.name, process.project?.root, process.cwd, process.pid, ...process.listeners.map(listener => listener.port)].filter(value => value !== null && value !== undefined).join(' ').toLowerCase().includes(normalized);
  const filtered = processes.filter(matches);
  const regular = filtered.filter(process => process.category !== 'protected').sort((a, b) => Number(b.canClose) - Number(a.canClose) || (a.project?.name ?? a.name).localeCompare(b.project?.name ?? b.name));
  const protectedProcesses = filtered.filter(process => process.category === 'protected');
  const busy = preparing || closing || plan !== null || forceConfirm;
  const portCount = new Set(filtered.flatMap(process => process.listeners.map(listener => listener.port))).size;
  return <main className="panel">
    <header className="toolbar"><div className="brand"><BrandMark /><div><h1>PortNest</h1><p>让开发端口各归其位</p></div></div><div className="toolbar-actions"><button className="icon-button" aria-label="刷新端口" title="刷新端口" disabled={loading || busy} onClick={() => void scan()}><RefreshCw size={17} className={loading ? 'spin' : ''} /></button><button className="icon-button" aria-label="打开设置" title="设置" onClick={() => report(api.settings())}><Settings size={17} /></button><button className="icon-button" aria-label="隐藏面板" title="隐藏面板" disabled={busy} onClick={hidePanel}><X size={17} /></button></div></header>
    <div className="search-wrap"><Search size={17} /><input type="search" aria-label="搜索端口、进程或项目" placeholder="搜索端口、进程或项目" value={query} onChange={event => setQuery(event.target.value)} autoComplete="off" spellCheck={false} />{query && <button className="icon-button" aria-label="清空搜索" onClick={() => setQuery('')}><X size={14} /></button>}</div>
    <div className="list-meta"><span><strong>{normalized ? regular.length + protectedProcesses.length : processes.length}</strong> 个进程<span className="dot">·</span>{portCount} 个 TCP 端口</span><span className="refresh-status" role="status">{preparing ? '正在核验…' : loading ? '正在扫描…' : snapshot ? `更新于 ${formatTime(snapshot.scannedAt)}` : '等待扫描'}</span></div>
    <div className="list-content">
      {error && <div className="error-banner" role="alert"><div><strong>{snapshot ? '刷新失败，正在显示上次结果' : '无法读取监听端口'}</strong><p>{error}</p>{snapshot && <p>结果时间：{formatTime(snapshot.scannedAt)}</p>}</div><button className="text-button" disabled={loading || busy} onClick={() => void scan()}>重试</button></div>}
      {actionError && <div className="error-banner" role="alert"><span>{actionError}</span><button className="icon-button" aria-label="关闭错误提示" onClick={() => setActionError(null)}><X size={14} /></button></div>}
      {outcome && <div className={`outcome-banner ${outcome.result.status === 'closed' ? 'success' : 'warning'}`} role="status"><strong>{outcome.result.status === 'closed' ? '进程已关闭' : '进程仍在监听'}</strong><p>{outcome.result.message}</p>{outcome.result.remainingPorts.length > 0 && <p>剩余端口：{outcome.result.remainingPorts.join('、')}</p>}{outcome.result.forceToken && <button className="force-link" disabled={busy || loading} onClick={() => { setActionError(null); setForceConfirm(true); }}>确认强制关闭</button>}</div>}
      {!snapshot && loading && <div className="empty-state" role="status"><LoaderCircle size={28} className="spin" /><strong>正在整理本机端口</strong><p>读取 TCP 监听与进程信息。</p></div>}
      {snapshot && filtered.length === 0 && <div className="empty-state"><Search size={28} /><strong>{normalized ? '没有匹配的进程' : '没有监听服务'}</strong><p>{normalized ? '试试其他端口号、进程名或项目名。' : '启动开发服务后，端口会出现在这里。'}</p>{normalized && <button className="secondary" onClick={() => setQuery('')}>清空搜索</button>}</div>}
      {regular.length > 0 && <section aria-label="开发与未知归属进程"><div className="section-label">开发与其他服务</div>{regular.map(process => <ProcessRow key={process.id} process={process} busy={busy} onClose={process => void prepare(process)} onOpen={(pid, port) => report(api.open(pid, port))} />)}</section>}
      {protectedProcesses.length > 0 && <section className="protected-section" aria-label="受保护进程"><button className="protected-disclosure" aria-expanded={showProtected || !!normalized} onClick={() => setShowProtected(value => !value)}><ShieldCheck size={15} /><span>受保护进程</span><span className="count-chip">{protectedProcesses.length}</span>{showProtected || normalized ? <ChevronDown size={15} /> : <ChevronRight size={15} />}</button>{(showProtected || !!normalized) && protectedProcesses.map(process => <ProcessRow key={process.id} process={process} busy={busy} onClose={process => void prepare(process)} onOpen={(pid, port) => report(api.open(pid, port))} />)}</section>}
      {snapshot?.limitations.length ? <div className="scan-limitations"><strong>扫描说明</strong>{snapshot.limitations.map((limitation, index) => <p key={index}>{limitation}</p>)}</div> : null}
    </div>
    <footer className="panel-footer"><span className="status-dot" /><span>仅本机处理</span><span className="footer-divider">·</span><span>展开时每 5 秒刷新</span></footer>
    {plan && <Dialog title={`关闭 ${plan.process.name}？`} busy={closing} onCancel={() => setPlan(null)} onConfirm={() => void execute(false)} confirmLabel="正常关闭">
      <p>将请求此进程退出，并等待最多 5 秒。以下全部端口会受到影响：</p><div className="dialog-ports">{plan.process.listeners.map(listener => <code key={listener.port}>{listener.port}</code>)}</div><dl className="dialog-process"><dt>进程</dt><dd>PID {plan.process.pid} · {plan.process.name}</dd><dt>项目</dt><dd>{plan.process.project?.root ?? '未知归属'}</dd><dt>工作目录</dt><dd>{plan.process.cwd ?? '无法读取'}</dd></dl>
      {plan.process.category === 'unknown' && <p className="warning-text">项目归属未知，请确认这是你要关闭的进程。</p>}<p className="muted">只关闭此进程，不递归关闭子进程。执行前会再次核验身份与端口。</p>
    </Dialog>}
    {forceConfirm && outcome && <Dialog title={`强制关闭 ${outcome.process.name}？`} danger busy={closing} onCancel={() => setForceConfirm(false)} onConfirm={() => void execute(true)} confirmLabel="强制关闭进程">
      <p>正常关闭后仍在监听。强制结束将跳过清理，可能丢失尚未保存的数据。</p><div className="dialog-ports">{outcome.result.remainingPorts.map(port => <code key={port}>{port}</code>)}</div><dl className="dialog-process"><dt>进程</dt><dd>PID {outcome.process.pid} · {outcome.process.name}</dd><dt>项目</dt><dd>{outcome.process.project?.root ?? '未知归属'}</dd></dl><p className="muted">剩余全部端口会受到影响。执行前会重新核验，身份或监听范围变化时中止。</p>
    </Dialog>}
  </main>;
}

export default function App() { return isSettingsWindow() ? <SettingsView /> : <Panel />; }
