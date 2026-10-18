import { ref } from 'vue';

export type DirectExtractAction = 'extract-here' | 'extract-folder';
export interface DirectExtractRequest { action: DirectExtractAction; paths: string[] }
export interface ShellRequest { action: 'pack' | 'unpack' | 'pack-same-name' | DirectExtractAction; paths: string[] }
const packPaths = ref<string[]>([]);
const unpackPaths = ref<string[]>([]);
const directRequests = ref<DirectExtractRequest[]>([]);
const quickPackRequests = ref<string[][]>([]);
const directActive = ref(false), quickActive = ref(false);
const pathKey = (path: string) => path.replaceAll('/', '\\').toLowerCase();
export function appendUniquePaths(existing: string[], incoming: string[]): string[] {
  const seen = new Set(existing.map(pathKey));
  return [...existing, ...incoming.filter(path => { const key = pathKey(path); if (seen.has(key)) return false; seen.add(key); return true; })];
}
export function enqueueDirectRequest(request: DirectExtractRequest) {
  const last = directRequests.value[directRequests.value.length - 1];
  // Explorer may deliver one invocation per selected path; coalesce pending siblings.
  if (last?.action === request.action) last.paths = appendUniquePaths(last.paths, request.paths);
  else directRequests.value.push({ action: request.action, paths: appendUniquePaths([], request.paths) });
}
export function enqueueQuickPack(paths: string[]) {
  const last = quickPackRequests.value[quickPackRequests.value.length - 1];
  if (last) quickPackRequests.value[quickPackRequests.value.length - 1] = appendUniquePaths(last, paths);
  else quickPackRequests.value.push(appendUniquePaths([], paths));
}
export function useShellQueue() { return { packPaths, unpackPaths, directRequests, quickPackRequests, directActive, quickActive }; }
