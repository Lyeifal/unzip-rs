import { ref } from 'vue';
import type { GameMetadata } from '../types';

export type LibraryFieldType = 'text' | 'select' | 'checkbox';
export interface LibraryField {
  id: string;
  label: string;
  type: LibraryFieldType;
  options: string[];
}

const KEY = 'archive_library_fields_v1';
const defaults: LibraryField[] = [
  { id: 'category', label: '分类', type: 'select', options: ['文档', '图片', '视频', '软件', '其他'] },
  { id: 'organized', label: '已整理', type: 'checkbox', options: [] },
];
const fields = ref<LibraryField[]>(defaults.map(f => ({ ...f, options: [...f.options] })));
const storageError = ref('');
const showPasswords = ref(false);
const legacyImported = new Set<string>();
const validId = (id: unknown): id is string => typeof id === 'string' && /^[a-z][a-z0-9_-]{0,79}$/i.test(id) && !['__proto__', 'constructor', 'prototype'].includes(id);
const optionsOf = (values: unknown): string[] => Array.isArray(values) ? [...new Set(values.filter((v): v is string => typeof v === 'string').map(v => v.trim()).filter(Boolean))] : [];

try {
  const raw = localStorage.getItem(KEY);
  if (raw) {
    const saved = JSON.parse(raw);
    showPasswords.value = saved.showPasswords === true;
    if (Array.isArray(saved.fields)) {
      const ids = new Set<string>();
      fields.value = saved.fields.filter((f: LibraryField) => {
        if (!f || !validId(f.id) || ids.has(f.id) || typeof f.label !== 'string' || !f.label.trim() || !['text', 'select', 'checkbox'].includes(f.type)) return false;
        ids.add(f.id); return true;
      }).map((f: LibraryField) => ({ id: f.id, label: f.label.trim(), type: f.type, options: optionsOf(f.options) }));
    }
    for (const id of saved.legacyImported ?? []) if (validId(id)) legacyImported.add(id);
  }
} catch { storageError.value = '字段设置读取失败，当前使用默认字段。'; }

function persist() {
  try {
    localStorage.setItem(KEY, JSON.stringify({ fields: fields.value, legacyImported: [...legacyImported], showPasswords: showPasswords.value }));
    storageError.value = '';
  } catch { storageError.value = '字段设置保存失败，请检查本机存储空间。'; }
}

function addField(type: LibraryFieldType) {
  const id = `field_${crypto.randomUUID().replaceAll('-', '')}`;
  fields.value.push({ id, label: '新字段', type, options: type === 'select' ? ['选项一', '选项二'] : [] });
  persist();
}
function updateField(id: string, patch: Partial<Pick<LibraryField, 'label' | 'options'>>) {
  const field = fields.value.find(f => f.id === id);
  if (!field) return;
  if (patch.label !== undefined && patch.label.trim()) field.label = patch.label.trim();
  if (patch.options !== undefined) field.options = optionsOf(patch.options);
  persist();
}
function removeField(id: string) {
  fields.value = fields.value.filter(f => f.id !== id);
  persist();
}
function moveField(id: string, delta: number) {
  const index = fields.value.findIndex(f => f.id === id), next = index + delta;
  if (index < 0 || next < 0 || next >= fields.value.length) return;
  [fields.value[index], fields.value[next]] = [fields.value[next], fields.value[index]];
  persist();
}
function importLegacy(field: LibraryField) {
  if (legacyImported.has(field.id)) return;
  legacyImported.add(field.id);
  if (!fields.value.some(f => f.id === field.id)) fields.value.push(field);
  persist();
}
function normalizeMetadata(source?: GameMetadata): GameMetadata {
  const metadata = { ...source };
  const values: Record<string, string | boolean> = {};
  for (const [key, value] of Object.entries(metadata.custom_fields ?? {})) {
    if (validId(key) && (typeof value === 'string' || typeof value === 'boolean')) values[key] = value;
  }
  if ((metadata.schema_version ?? 0) < 1) {
    metadata.notes = [metadata.notes?.trim(), metadata.version?.trim()].filter(Boolean).join('\n');
    if (metadata.engine) {
      values.legacy_engine ??= metadata.engine;
      importLegacy({ id: 'legacy_engine', label: '引擎', type: 'select', options: ['Unity', 'Unreal', 'RPG Maker', 'Godot', '自研', '其他'] });
    }
    if (metadata.is_apk === true) {
      values.legacy_apk ??= true;
      importLegacy({ id: 'legacy_apk', label: 'APK 安装包', type: 'checkbox', options: [] });
    }
    metadata.schema_version = 1;
  }
  metadata.custom_fields = values;
  return metadata;
}

export function useLibraryFields() {
  return { fields, storageError, showPasswords, setShowPasswords: (value: boolean) => { showPasswords.value = value; persist(); }, addField, updateField, removeField, moveField, normalizeMetadata };
}
