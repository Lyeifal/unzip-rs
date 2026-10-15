<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { SelectOption } from "../types/select";

const props = defineProps<{
  modelValue: string | number;
  options: SelectOption[];
  ariaLabel: string;
  placeholder?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string | number];
}>();

let nextId = 0;
const listId = `styled-select-${++nextId}`;
const root = ref<HTMLElement | null>(null);
const open = ref(false);
const openAbove = ref(false);
const activeIndex = ref(-1);

const selectedIndex = computed(() =>
  props.options.findIndex((option) => option.value === props.modelValue)
);
const selectedLabel = computed(() =>
  props.options[selectedIndex.value]?.label ?? props.placeholder ?? "请选择"
);

function showMenu() {
  open.value = true;
  activeIndex.value = selectedIndex.value >= 0 ? selectedIndex.value : 0;
  const bounds = root.value?.getBoundingClientRect();
  const menuHeight = Math.min(260, props.options.length * 38 + 8);
  openAbove.value = !!bounds && window.innerHeight - bounds.bottom < menuHeight && bounds.top > menuHeight;
}

function choose(index: number) {
  const option = props.options[index];
  if (!option) return;
  emit("update:modelValue", option.value);
  open.value = false;
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
  } else if ((event.key === "Enter" || event.key === " ") && open.value) {
    event.preventDefault();
    choose(activeIndex.value);
  } else if (event.key === "Escape" && open.value) {
    event.preventDefault();
    open.value = false;
  } else if (event.key === "Tab") {
    open.value = false;
  }
}

function onDocumentPointerDown(event: PointerEvent) {
  if (root.value && !root.value.contains(event.target as Node)) open.value = false;
}

onMounted(() => document.addEventListener("pointerdown", onDocumentPointerDown));
onUnmounted(() => document.removeEventListener("pointerdown", onDocumentPointerDown));
</script>

<template>
  <div ref="root" class="select-field">
    <button
      type="button"
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
    <div
      v-if="open"
      :id="listId"
      class="select-menu"
      :class="{ above: openAbove }"
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
        :aria-selected="index === selectedIndex"
        @mouseenter="activeIndex = index"
        @click="choose(index)"
      >
        {{ option.label }}
      </button>
    </div>
  </div>
</template>
