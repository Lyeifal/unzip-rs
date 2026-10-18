import { invoke } from '@tauri-apps/api/core';
import type { Config } from '../types';

export type ToolPaths = Pick<Config, 'seven_zip' | 'unrar' | 'lz4'>;
let pendingSave: Promise<ToolPaths> | undefined;

export async function saveToolPaths(paths: ToolPaths): Promise<ToolPaths> {
  if (pendingSave) throw new Error('工具路径正在保存，请稍候');
  const request = invoke<ToolPaths>('save_tool_paths', { paths: { ...paths } });
  pendingSave = request;
  try { return await request; }
  finally { if (pendingSave === request) pendingSave = undefined; }
}

// A task started immediately after closing settings must wait for that save.
export async function waitForToolPathsSave(): Promise<void> {
  if (pendingSave) await pendingSave;
}
