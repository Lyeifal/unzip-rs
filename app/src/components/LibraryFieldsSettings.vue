<script setup lang="ts">
import { useLibraryFields, type LibraryFieldType } from '../composables/useLibraryFields';
const library = useLibraryFields();
const typeLabels: Record<LibraryFieldType, string> = { text: '文本', select: '下拉选项', checkbox: '勾选项' };
function rename(id: string, event: Event) {
  const input = event.target as HTMLInputElement;
  library.updateField(id, { label: input.value });
  input.value = library.fields.value.find(f => f.id === id)?.label ?? '';
}
</script>

<template>
  <section class="library-fields-settings">
    <h3>压缩包管理</h3>
    <label class="password-setting">
      <input type="checkbox" :checked="library.showPasswords.value" @change="library.setShowPasswords(($event.target as HTMLInputElement).checked)" />
      显示压缩包密码
    </label>
    <p>默认隐藏。此开关仅影响卡片显示，记录和导出的备份仍包含密码。</p>
    <h3>自定义字段</h3>
    <p>备注始终保留。可添加自己的分类、状态等词条；移除字段不会删除条目中已保存的值。</p>
    <div v-for="(field, index) in library.fields.value" :key="field.id" class="field-editor">
      <div class="field-heading">
        <input class="input-glass" :value="field.label" :aria-label="`字段名称 ${field.label}`" @change="rename(field.id, $event)" />
        <span class="field-type">{{ typeLabels[field.type] }}</span>
        <button class="btn-glass" :disabled="index === 0" aria-label="上移字段" @click="library.moveField(field.id, -1)">↑</button>
        <button class="btn-glass" :disabled="index === library.fields.value.length - 1" aria-label="下移字段" @click="library.moveField(field.id, 1)">↓</button>
        <button class="btn-glass" :aria-label="`移除字段 ${field.label}`" @click="library.removeField(field.id)">移除</button>
      </div>
      <label v-if="field.type === 'select'" class="options-editor">
        <span>选项（每行一个）</span>
        <textarea class="input-glass" :value="field.options.join('\n')" :aria-label="`${field.label}的选项`" rows="3" @change="library.updateField(field.id, { options: ($event.target as HTMLTextAreaElement).value.split('\n') })" />
      </label>
    </div>
    <div class="field-add-actions">
      <button v-for="(label, type) in typeLabels" :key="type" class="btn-glass" @click="library.addField(type)">＋ {{ label }}</button>
    </div>
    <p v-if="library.storageError.value" class="field-error" role="alert">{{ library.storageError.value }}</p>
  </section>
</template>

<style scoped>
h3 { margin: 0 0 10px; font-size: 16px; }
.password-setting { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--text-primary); }
p, .options-editor > span { color: var(--text-secondary); font-size: 12px; line-height: 1.6; }
.field-editor { margin: 12px 0; padding: 12px; border: 1px solid var(--border-subtle); border-radius: var(--radius-md); background: var(--surface-input); }
.field-heading, .field-add-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; }
.field-heading input { flex: 1; min-width: 100px; width: 100px; }
.field-heading button { padding: 6px 9px; }
.field-type { font-size: 12px; color: var(--text-secondary); }
.options-editor { display: grid; gap: 6px; margin-top: 10px; }
textarea { width: 100%; resize: vertical; }
.field-error { color: var(--error); }
</style>
