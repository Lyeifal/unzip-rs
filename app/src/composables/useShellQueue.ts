import { ref } from 'vue';

export interface ShellRequest { action: 'pack' | 'unpack'; paths: string[] }
const packPaths = ref<string[]>([]);
const unpackPaths = ref<string[]>([]);
const pathKey = (path: string) => path.replaceAll('/', '\\').toLowerCase();
export function appendUniquePaths(existing: string[], incoming: string[]): string[] {
  const seen = new Set(existing.map(pathKey));
  return [...existing, ...incoming.filter(path => { const key = pathKey(path); if (seen.has(key)) return false; seen.add(key); return true; })];
}
export function useShellQueue() { return { packPaths, unpackPaths }; }
