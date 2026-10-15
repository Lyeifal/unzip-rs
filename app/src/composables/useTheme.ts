/**
 * 主题管理 Composable
 * 管理壁纸、配色方案、主题预设与持久化
 */

import { ref } from 'vue';
import {
  extractColorsFromImage,
  applyColorPalette,
  darken,
  getLuminance,
  resetToDefaultPalette,
  type ColorPalette,
} from '../utils/colorExtractor';
import {
  THEME_PRESETS,
  DEFAULT_PRESET_ID,
  SYSTEM_DARK_PALETTE,
  SYSTEM_LIGHT_PALETTE,
  getPreset,
  isThemePresetId,
  type ThemeMode,
  type ThemePresetId,
} from '../utils/themePresets';

export interface WallpaperConfig {
  enabled: boolean;
  imageData: string;      // Base64 编码的图片
  autoTheme: boolean;     // 兼容别名，等价于 themeMode === 'wallpaper'
  themeMode: ThemeMode;
  presetId: ThemePresetId;
  palette?: ColorPalette; // 提取的配色
  manualColors: ManualThemeColors;
  cardOpacity: number;
  blur: number;           // 模糊程度 (0-20px)
  darkness: number;       // 暗度 (0-1)
}

export interface ManualThemeColors {
  primary: string;
  secondary: string;
  accent: string;
  background: string;
}

export type { ThemeMode, ThemePresetId };
export { THEME_PRESETS };

const DEFAULT_MANUAL_COLORS: ManualThemeColors = {
  primary: '#83AD9D',
  secondary: '#9AAFA6',
  accent: '#D7B77C',
  background: '#191E1E',
};

const STORAGE_KEY = 'unzip_wallpaper_config';

function createDefaultConfig(): WallpaperConfig {
  // 全新安装默认：system 预设
  return {
    enabled: false,
    imageData: '',
    autoTheme: false,
    themeMode: 'preset',
    presetId: DEFAULT_PRESET_ID,
    manualColors: { ...DEFAULT_MANUAL_COLORS },
    cardOpacity: 0.92,
    blur: 0,
    darkness: 0.4,
  };
}

// 全局状态
const wallpaperConfig = ref<WallpaperConfig>(createDefaultConfig());

const isLoading = ref(false);
const error = ref<string | null>(null);

/**
 * 保存配置到 localStorage
 */
function saveConfig() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(wallpaperConfig.value));
  } catch (e) {
    console.error('保存壁纸配置失败：', e);
  }
}

/**
 * 从 localStorage 加载配置
 */
function loadConfig(): WallpaperConfig | null {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    return saved ? JSON.parse(saved) : null;
  } catch (e) {
    console.error('加载壁纸配置失败：', e);
    return null;
  }
}

/**
 * 应用壁纸到页面
 */
function applyWallpaper(imageData: string, blur: number, darkness: number) {
  document.body.style.setProperty('--user-wallpaper', `url(${imageData})`);
  document.body.style.setProperty('--wallpaper-blur', `${blur}px`);
  document.body.style.setProperty('--wallpaper-darkness', `${darkness}`);
  document.body.classList.add('wallpaper-loaded');
}

/**
 * 移除壁纸
 */
function removeWallpaper() {
  document.body.style.removeProperty('--user-wallpaper');
  document.body.classList.remove('wallpaper-loaded');
}

function applyManualColors(colors: ManualThemeColors) {
  applyColorPalette({
    ...colors,
    text: getLuminance(colors.background) > 0.52 ? '#17201C' : '#E6EAE7',
  });
}

function normalizeAutoPalette(palette: ColorPalette): ColorPalette {
  const background = getLuminance(palette.background) > 0.52
    ? darken(palette.background, 0.48)
    : palette.background;
  return {
    ...palette,
    background,
    text: getLuminance(background) > 0.52 ? '#17201C' : '#E6EAE7',
  };
}

function applyCardOpacity(opacity: number) {
  document.documentElement.style.setProperty(
    '--card-opacity',
    `${Math.round(opacity * 100)}%`
  );
}

/** 让 autoTheme 始终作为 themeMode === 'wallpaper' 的兼容别名 */
function syncModeAlias(config: WallpaperConfig): void {
  config.autoTheme = config.themeMode === 'wallpaper';
}

/**
 * 规范化旧版本保存的配置：
 * 旧版只有 autoTheme，true => wallpaper 模式，false => manual 模式
 */
function normalizeSavedConfig(saved: Partial<WallpaperConfig>): WallpaperConfig {
  const config: WallpaperConfig = {
    ...createDefaultConfig(),
    ...saved,
    manualColors: { ...DEFAULT_MANUAL_COLORS, ...saved.manualColors },
    cardOpacity: saved.cardOpacity ?? 0.92,
    blur: saved.blur ?? 0,
    darkness: saved.darkness ?? 0.4,
  };

  if (!saved.themeMode || !['preset', 'wallpaper', 'manual'].includes(saved.themeMode)) {
    config.themeMode = typeof saved.autoTheme === 'boolean'
      ? (saved.autoTheme ? 'wallpaper' : 'manual')
      : 'preset';
  }
  if (!isThemePresetId(config.presetId)) {
    config.presetId = DEFAULT_PRESET_ID;
  }
  syncModeAlias(config);
  return config;
}

function systemPrefersDark(): boolean {
  return typeof window !== 'undefined' && 'matchMedia' in window
    ? window.matchMedia('(prefers-color-scheme: dark)').matches
    : true;
}

/** 解析预设配色；system 按当前 OS 外观解析 */
function resolvePresetPalette(id: ThemePresetId): ColorPalette {
  const preset = getPreset(id);
  if (preset.id === 'system') {
    return systemPrefersDark() ? SYSTEM_DARK_PALETTE : SYSTEM_LIGHT_PALETTE;
  }
  return preset.palette as ColorPalette;
}

function applyPreset(id: ThemePresetId): void {
  applyColorPalette(resolvePresetPalette(id));
}

let systemThemeListenerInstalled = false;

/**
 * 单例系统主题监听：OS 外观变化时，仅当 system 预设处于选中状态才切换
 */
function installSystemThemeListener(): void {
  if (systemThemeListenerInstalled) return;
  if (typeof window === 'undefined' || !('matchMedia' in window)) return;
  systemThemeListenerInstalled = true;

  const mql = window.matchMedia('(prefers-color-scheme: dark)');
  const handleChange = () => {
    if (
      wallpaperConfig.value.themeMode === 'preset' &&
      wallpaperConfig.value.presetId === 'system'
    ) {
      applyPreset('system');
    }
  };

  if (typeof mql.addEventListener === 'function') {
    mql.addEventListener('change', handleChange);
  } else {
    (mql as unknown as { addListener: (callback: () => void) => void }).addListener(handleChange);
  }
}

// 取色任务令牌：壁纸更换/移除时使旧任务过期，防止异步结果覆盖新选择
let extractionToken = 0;

/**
 * 用壁纸 dataURL 提取配色并应用（上传与重试共用同一 imageData）。
 * 失败时保留壁纸与原配置，仅记录精确错误。
 */
async function extractAndApplyPalette(imageData: string): Promise<void> {
  const token = ++extractionToken;
  isLoading.value = true;
  error.value = null;

  try {
    const palette = await extractColorsFromImage(imageData);
    if (token !== extractionToken) return;
    if (wallpaperConfig.value.imageData !== imageData) return;

    wallpaperConfig.value.palette = palette;
    // 只有用户当前处于壁纸取色模式时才应用，避免覆盖预设/手动选择
    if (wallpaperConfig.value.themeMode === 'wallpaper') {
      applyColorPalette(palette);
    }
    saveConfig();
  } catch (e) {
    if (token !== extractionToken) return;
    if (wallpaperConfig.value.themeMode !== 'wallpaper') return;
    const reason = e instanceof Error ? e.message : String(e);
    error.value = `自动取色失败：${reason}`;
    console.error(e);
  } finally {
    if (token === extractionToken) isLoading.value = false;
  }
}

/** 按当前模式应用配色 */
function applyModeColors(): void {
  const { themeMode, presetId, palette } = wallpaperConfig.value;
  if (themeMode === 'preset') {
    applyPreset(presetId);
  } else if (themeMode === 'wallpaper' && palette) {
    applyColorPalette(palette);
  } else {
    applyManualColors(wallpaperConfig.value.manualColors);
  }
}

export function useTheme() {
  /**
   * 上传壁纸
   */
  async function uploadWallpaper(file: File): Promise<void> {
    const uploadToken = ++extractionToken;
    isLoading.value = true;
    error.value = null;

    try {
      // 检查文件大小（限制 5MB）
      if (file.size > 5 * 1024 * 1024) {
        throw new Error('图片大小不能超过 5MB');
      }

      // 检查文件类型
      if (!file.type.startsWith('image/') && !/\.avif$/i.test(file.name)) {
        throw new Error('请选择图片文件');
      }

      // 转换为 Base64
      const reader = new FileReader();
      const imageData = await new Promise<string>((resolve, reject) => {
        reader.onload = () => resolve(reader.result as string);
        reader.onerror = () => reject(new Error('读取文件失败'));
        reader.onabort = () => reject(new Error('图片读取已取消'));
        reader.readAsDataURL(file);
      });

      if (uploadToken !== extractionToken) return;

      // 先应用壁纸。自动取色失败时也不应阻止图片显示。
      wallpaperConfig.value = {
        ...wallpaperConfig.value,
        enabled: true,
        imageData,
        palette: undefined,
      };
      syncModeAlias(wallpaperConfig.value);

      applyWallpaper(
        imageData,
        wallpaperConfig.value.blur,
        wallpaperConfig.value.darkness
      );
      saveConfig();

      // 单独提取配色；格式解码或 Canvas 取色失败不撤销已应用的壁纸。
      if (wallpaperConfig.value.autoTheme) {
        await extractAndApplyPalette(imageData);
      }
    } catch (e) {
      if (uploadToken !== extractionToken) return;
      error.value = e instanceof Error ? e.message : '上传失败';
      throw e;
    } finally {
      if (uploadToken === extractionToken) isLoading.value = false;
    }
  }

  /**
   * 移除壁纸
   */
  function clearWallpaper(): void {
    const { themeMode, presetId, manualColors, cardOpacity } = wallpaperConfig.value;
    wallpaperConfig.value = {
      ...wallpaperConfig.value,
      enabled: false,
      imageData: '',
      palette: undefined,
      blur: 0,
      darkness: 0.4,
    };
    syncModeAlias(wallpaperConfig.value);
    extractionToken += 1;
    isLoading.value = false;
    error.value = null;

    removeWallpaper();
    if (themeMode === 'preset') {
      applyPreset(presetId);
    } else if (themeMode === 'manual') {
      applyManualColors(manualColors);
    } else {
      resetToDefaultPalette();
    }
    applyCardOpacity(cardOpacity);
    saveConfig();
  }

  /**
   * 重新提取颜色（重试）
   */
  async function reextractColors(): Promise<void> {
    const { imageData } = wallpaperConfig.value;
    if (!imageData) {
      return;
    }

    await extractAndApplyPalette(imageData);
  }

  /**
   * 切换自动取色（壁纸模式）
   */
  function toggleAutoTheme(enabled: boolean): void {
    wallpaperConfig.value.themeMode = enabled ? 'wallpaper' : 'manual';
    syncModeAlias(wallpaperConfig.value);

    if (enabled) {
      const { imageData, palette } = wallpaperConfig.value;
      if (palette) {
        applyColorPalette(palette);
      } else if (imageData) {
        // 已有壁纸但没有配色：立即补提取，而不是无响应
        void extractAndApplyPalette(imageData);
      }
    } else {
      applyManualColors(wallpaperConfig.value.manualColors);
    }

    saveConfig();
  }

  /**
   * 选择主题预设
   */
  function selectPreset(id: ThemePresetId): void {
    error.value = null;
    wallpaperConfig.value.themeMode = 'preset';
    wallpaperConfig.value.presetId = id;
    syncModeAlias(wallpaperConfig.value);

    applyPreset(id);
    saveConfig();
  }

  function updateManualColor(key: keyof ManualThemeColors, color: string): void {
    error.value = null;
    wallpaperConfig.value.manualColors[key] = color;
    // 手动改色即进入 manual 模式
    wallpaperConfig.value.themeMode = 'manual';
    syncModeAlias(wallpaperConfig.value);
    applyManualColors(wallpaperConfig.value.manualColors);
    saveConfig();
  }

  function updateCardOpacity(opacity: number): void {
    wallpaperConfig.value.cardOpacity = Math.max(0.55, Math.min(1, opacity));
    applyCardOpacity(wallpaperConfig.value.cardOpacity);
    saveConfig();
  }

  /**
   * 更新模糊度
   */
  function updateBlur(blur: number): void {
    wallpaperConfig.value.blur = blur;
    if (wallpaperConfig.value.enabled) {
      document.body.style.setProperty('--wallpaper-blur', `${blur}px`);
    }
    saveConfig();
  }

  /**
   * 更新暗度
   */
  function updateDarkness(darkness: number): void {
    wallpaperConfig.value.darkness = darkness;
    if (wallpaperConfig.value.enabled) {
      document.body.style.setProperty('--wallpaper-darkness', `${darkness}`);
    }
    saveConfig();
  }

  /**
   * 初始化（恢复配置）
   */
  function init(): void {
    installSystemThemeListener();

    const saved = loadConfig();
    if (saved) {
      const config = normalizeSavedConfig(saved);
      wallpaperConfig.value = config;

      if (config.enabled && config.imageData) {
        applyWallpaper(config.imageData, config.blur, config.darkness);
      }

      if (config.themeMode === 'wallpaper' && config.palette) {
        // 旧版取色可能留下过亮的背景，统一收敛为深色底
        wallpaperConfig.value.palette = normalizeAutoPalette(config.palette);
      }

      applyModeColors();
      applyCardOpacity(config.cardOpacity);
      saveConfig();

      // 壁纸模式下有图却无配色（例如旧版取色失败后保存）：启动时自动补提取
      if (
        config.themeMode === 'wallpaper' &&
        config.enabled &&
        config.imageData &&
        !wallpaperConfig.value.palette
      ) {
        void extractAndApplyPalette(config.imageData);
      }
    } else {
      // 全新安装默认：system 预设
      applyPreset(wallpaperConfig.value.presetId);
      applyCardOpacity(wallpaperConfig.value.cardOpacity);
      saveConfig();
    }
  }

  return {
    config: wallpaperConfig,
    isLoading,
    error,
    uploadWallpaper,
    clearWallpaper,
    reextractColors,
    toggleAutoTheme,
    selectPreset,
    updateManualColor,
    updateCardOpacity,
    updateBlur,
    updateDarkness,
    init,
  };
}
