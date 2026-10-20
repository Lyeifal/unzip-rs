<script setup lang="ts">
import { computed, nextTick, onDeactivated, onMounted, onUnmounted, ref, useId, watch, type CSSProperties } from "vue";
import type { SelectOption } from "../types/select";

const props = defineProps<{
  modelValue: string | number;
  options: SelectOption[];
  ariaLabel: string;
  placeholder?: string;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string | number];
}>();

const listId = `styled-select-${useId()}`;
const root = ref<HTMLElement | null>(null);
const menu = ref<HTMLElement | null>(null);
const control = ref<HTMLButtonElement | null>(null);
const open = ref(false);
const menuStyle = ref<CSSProperties>({});
const activeIndex = ref(-1);

const selectedIndex = computed(() =>
  props.options.findIndex((option) => option.value === props.modelValue)
);
const selectedLabel = computed(() =>
  props.options[selectedIndex.value]?.label ?? props.placeholder ?? "请选择"
);

function showMenu() {
  if (props.disabled || control.value?.matches(':disabled') || !props.options.length) return;
  open.value = true;
  activeIndex.value = selectedIndex.value >= 0 ? selectedIndex.value : 0;
  positionMenu();
}

function positionMenu() {
  const bounds = root.value?.getBoundingClientRect();
  if (!bounds) return;
  const below = window.innerHeight - bounds.bottom - 10;
  const above = bounds.top - 10;
  const openAbove = below < Math.min(260, props.options.length * 38 + 8) && above > below;
  const width = Math.min(bounds.width, window.innerWidth - 16);
  menuStyle.value = {
    position: 'fixed',
    left: `${Math.max(8, Math.min(bounds.left, window.innerWidth - width - 8))}px`,
    width: `${width}px`,
    top: openAbove ? 'auto' : `${bounds.bottom + 5}px`,
    bottom: openAbove ? `${window.innerHeight - bounds.top + 5}px` : 'auto',
    maxHeight: `${Math.max(0, Math.min(260, openAbove ? above : below))}px`,
  };
}

function choose(index: number) {
  const option = props.options[index];
  if (!option || props.disabled || control.value?.matches(':disabled')) return;
  emit("update:modelValue", option.value);
  open.value = false;
  control.value?.focus({ preventScroll: true });
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    if (!open.value) {
      showMenu();
      return;
    }
    const delta = event.key === "ArrowDown" ? 1 : -1;
    activeIndex.value = (activeIndex.value + delta + props.options.length) % props.options.length;
  } else if (event.key === "Home" && open.value) {
    event.preventDefault();
    activeIndex.value = 0;
  } else if (event.key === "End" && open.value) {
    event.preventDefault();
    activeIndex.value = props.options.length - 1;
  } else if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    if (open.value) choose(activeIndex.value);
    else showMenu();
  } else if (event.key === "Escape" && open.value) {
    event.preventDefault();
    event.stopPropagation();
    open.value = false;
  } else if (event.key === "Tab") {
    open.value = false;
  }
}

function onDocumentPointerDown(event: PointerEvent) {
  if (!root.value?.contains(event.target as Node) && !menu.value?.contains(event.target as Node)) open.value = false;
}

function onContainerScroll(event: Event) {
  if (open.value && !menu.value?.contains(event.target as Node)) positionMenu();
}
function closeMenu() { open.value = false; }
watch([open, activeIndex], async () => {
  if (!open.value) return;
  await nextTick();
  menu.value?.querySelector('.select-option.active')?.scrollIntoView({ block: 'nearest' });
});
watch(() => [props.disabled, props.options], () => {
  if (props.disabled || !props.options.length) open.value = false;
  activeIndex.value = Math.min(activeIndex.value, props.options.length - 1);
});
onMounted(() => {
  document.addEventListener("pointerdown", onDocumentPointerDown);
  window.addEventListener('scroll', onContainerScroll, true);
  window.addEventListener('resize', closeMenu);
});
onDeactivated(closeMenu);
onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown);
  window.removeEventListener('scroll', onContainerScroll, true);
  window.removeEventListener('resize', closeMenu);
});
</script>

<template>
  <div ref="root" class="select-field">
    <button
      ref="control"
      type="button"
      :disabled="disabled"
      class="select-control"
      role="combobox"
      aria-haspopup="listbox"
      :aria-label="ariaLabel"
      :aria-expanded="open"
      :aria-controls="listId"
      :aria-activedescendant="open ? `${listId}-option-${activeIndex}` : undefined"
      @click="open ? (open = false) : showMenu()"
      @keydown="onKeydown"
    >
      <span class="select-value">{{ selectedLabel }}</span>
      <span class="select-chevron" aria-hidden="true"></span>
    </button>
    <Teleport to="body">
    <div
      v-if="open"
      ref="menu"
      :id="listId"
      class="select-menu"
      :style="menuStyle"
      role="listbox"
      :aria-label="ariaLabel"
    >
      <button
        v-for="(option, index) in options"
        :id="`${listId}-option-${index}`"
        :key="`${option.value}`"
        type="button"
        class="select-option"
        :class="{ active: index === activeIndex, selected: index === selectedIndex }"
        role="option"
        tabindex="-1"
        :aria-selected="index === selectedIndex"
        @mouseenter="activeIndex = index"
        @pointerdown.prevent
        @click="choose(index)"
      >
        {{ option.label }}
      </button>
    </div>
    </Teleport>
  </div>
</template>
