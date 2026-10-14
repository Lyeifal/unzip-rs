/**
 * 主题管理 Composable
 * 管理壁纸、配色方案、持久化
 */

import { ref, watch } from 'vue';
import {
  extractColorsFromImage,
  applyColorPalette,
  resetToDefaultPalette,
  type ColorPalette,
} from '../utils/colorExtractor';

export interface WallpaperConfig {
  enabled: boolean;
  imageData: string;      // Base64 编码的图片
  autoTheme: boolean;     // 是否自动取色
  palette?: ColorPalette; // 提取的配色
  blur: number;           // 模糊程度 (0-20px)
  darkness: number;       // 暗度 (0-1)
}

const STORAGE_KEY = 'unzip_wallpaper_config';

// 全局状态
const wallpaperConfig = ref<WallpaperConfig>({
  enabled: false,
  imageData: '',
  autoTheme: true,
  blur: 0,
  darkness: 0.4,
});

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

export function useTheme() {
  /**
   * 上传壁纸
   */
  async function uploadWallpaper(file: File): Promise<void> {
    isLoading.value = true;
    error.value = null;

    try {
      // 检查文件大小（限制 5MB）
      if (file.size > 5 * 1024 * 1024) {
        throw new Error('图片大小不能超过 5MB');
      }

      // 检查文件类型
      if (!file.type.startsWith('image/')) {
        throw new Error('请选择图片文件');
      }

      // 转换为 Base64
      const reader = new FileReader();
      const imageData = await new Promise<string>((resolve, reject) => {
        reader.onload = () => resolve(reader.result as string);
        reader.onerror = () => reject(new Error('读取文件失败'));
        reader.readAsDataURL(file);
      });

      // 提取配色（如果启用自动取色）
      let palette: ColorPalette | undefined;
      if (wallpaperConfig.value.autoTheme) {
        palette = await extractColorsFromImage(file);
        applyColorPalette(palette);
      }

      // 更新配置
      wallpaperConfig.value = {
        ...wallpaperConfig.value,
        enabled: true,
        imageData,
        palette,
      };

      // 应用壁纸
      applyWallpaper(
        imageData,
        wallpaperConfig.value.blur,
        wallpaperConfig.value.darkness
      );

      saveConfig();
    } catch (e) {
      error.value = e instanceof Error ? e.message : '上传失败';
      throw e;
    } finally {
      isLoading.value = false;
    }
  }

  /**
   * 移除壁纸
   */
  function clearWallpaper(): void {
    wallpaperConfig.value = {
      enabled: false,
      imageData: '',
      autoTheme: true,
      blur: 0,
      darkness: 0.4,
    };

    removeWallpaper();
    resetToDefaultPalette();
    saveConfig();
  }

  /**
   * 重新提取颜色
   */
  async function reextractColors(): Promise<void> {
    if (!wallpaperConfig.value.imageData) {
      return;
    }

    isLoading.value = true;
    error.value = null;

    try {
      // 将 Base64 转回 File
      const response = await fetch(wallpaperConfig.value.imageData);
      const blob = await response.blob();
      const file = new File([blob], 'wallpaper.jpg', { type: blob.type });

      // 重新提取配色
      const palette = await extractColorsFromImage(file);
      applyColorPalette(palette);

      wallpaperConfig.value.palette = palette;
      saveConfig();
    } catch (e) {
      error.value = '提取颜色失败';
      console.error(e);
    } finally {
      isLoading.value = false;
    }
  }

  /**
   * 切换自动取色
   */
  function toggleAutoTheme(enabled: boolean): void {
    wallpaperConfig.value.autoTheme = enabled;

    if (enabled && wallpaperConfig.value.palette) {
      applyColorPalette(wallpaperConfig.value.palette);
    } else if (!enabled) {
      resetToDefaultPalette();
    }

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
    const saved = loadConfig();
    if (saved) {
      wallpaperConfig.value = saved;

      if (saved.enabled && saved.imageData) {
        applyWallpaper(saved.imageData, saved.blur, saved.darkness);

        if (saved.autoTheme && saved.palette) {
          applyColorPalette(saved.palette);
        }
      }
    } else {
      // 应用默认配色
      resetToDefaultPalette();
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
    updateBlur,
    updateDarkness,
    init,
  };
}
