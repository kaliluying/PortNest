export interface Listener {
  port: number;
  addresses: string[];
  scope: 'local' | 'lan';
}

export interface Project {
  name: string;
  root: string;
  marker: string;
}

export interface ProcessRecord {
  id: string;
  pid: number;
  uid: number | null;
  name: string;
  executable: string | null;
  cwd: string | null;
  startedAt: number | null;
  project: Project | null;
  category: 'development' | 'unknown' | 'protected';
  protectionReason: string | null;
  canClose: boolean;
  listeners: Listener[];
}

export interface Snapshot {
  processes: ProcessRecord[];
  scannedAt: number;
  limitations: string[];
}

export interface ClosePlan {
  token: string;
  process: ProcessRecord;
  expiresAt: number;
}

export interface CloseOutcome {
  status: 'closed' | 'stillListening';
  pid: number;
  remainingPorts: number[];
  message: string;
  forceToken: string | null;
}

export interface Preferences {
  autostart: boolean;
}
