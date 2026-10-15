/**
 * 主题预设目录
 * 提供 system / 纯色 / Morandi 系列预设，配色保持低饱和、协调统一
 */

import type { ColorPalette } from './colorExtractor';

export type ThemeMode = 'preset' | 'wallpaper' | 'manual';

export type ThemePresetId =
  | 'system'
  | 'black'
  | 'white'
  | 'sage'
  | 'mist'
  | 'sand'
  | 'rose'
  | 'lavender';

export interface ThemePreset {
  id: ThemePresetId;
  label: string;       // 中文名称
  description: string; // 简短中文描述
  dark: boolean;       // 主基调（system 跟随系统）
  swatches: string[];  // 展示用色板（3-4 个）
  palette: ColorPalette | null; // system 为 null，运行时按 OS 解析
}

/** system 预设：深色分支（与默认深色调一致） */
export const SYSTEM_DARK_PALETTE: ColorPalette = {
  primary: '#83AD9D',
  secondary: '#9AAFA6',
  accent: '#D7B77C',
  background: '#191E1E',
  text: '#E6EAE7',
};

/** system 预设：浅色分支 */
export const SYSTEM_LIGHT_PALETTE: ColorPalette = {
  primary: '#5E7F72',
  secondary: '#7A8B84',
  accent: '#A8843F',
  background: '#F2F1EC',
  text: '#232826',
};

export const DEFAULT_PRESET_ID: ThemePresetId = 'system';

export const THEME_PRESETS: ThemePreset[] = [
  {
    id: 'system',
    label: '跟随系统',
    description: '自动跟随系统的浅色或深色外观',
    dark: false,
    swatches: ['#191E1E', '#F2F1EC', '#83AD9D', '#D7B77C'],
    palette: null,
  },
  {
    id: 'black',
    label: '墨黑',
    description: '沉稳的炭黑基调，冷灰点缀',
    dark: true,
    swatches: ['#141619', '#7D8A93', '#5F6B73', '#9C8F7E'],
    palette: {
      primary: '#7D8A93',
      secondary: '#5F6B73',
      accent: '#9C8F7E',
      background: '#141619',
      text: '#D8DBDD',
    },
  },
  {
    id: 'white',
    label: '暖白',
    description: '温润的米白底色，柔和护眼',
    dark: false,
    swatches: ['#F4F1EA', '#6F7F77', '#8B948C', '#A98F63'],
    palette: {
      primary: '#6F7F77',
      secondary: '#8B948C',
      accent: '#A98F63',
      background: '#F4F1EA',
      text: '#26241E',
    },
  },
  {
    id: 'sage',
    label: '灰鼠尾草',
    description: 'Morandi 深灰绿，安静克制',
    dark: true,
    swatches: ['#1D2420', '#8FAE9C', '#74857B', '#B3A27E'],
    palette: {
      primary: '#8FAE9C',
      secondary: '#74857B',
      accent: '#B3A27E',
      background: '#1D2420',
      text: '#E3E8E4',
    },
  },
  {
    id: 'mist',
    label: '薄雾蓝',
    description: '淡蓝灰如晨雾，清爽干净',
    dark: false,
    swatches: ['#E3E8EA', '#6C8899', '#8FA3AD', '#A58F72'],
    palette: {
      primary: '#6C8899',
      secondary: '#8FA3AD',
      accent: '#A58F72',
      background: '#E3E8EA',
      text: '#252C30',
    },
  },
  {
    id: 'sand',
    label: '燕麦沙',
    description: '温暖燕麦色，像午后阳光',
    dark: false,
    swatches: ['#EFE8DB', '#A08A6B', '#B5A488', '#7F8B6E'],
    palette: {
      primary: '#A08A6B',
      secondary: '#B5A488',
      accent: '#7F8B6E',
      background: '#EFE8DB',
      text: '#2E2A22',
    },
  },
  {
    id: 'rose',
    label: '灰玫瑰',
    description: '低饱和的干枯玫瑰粉',
    dark: false,
    swatches: ['#F0E7E3', '#A97F7C', '#BFA3A0', '#7E8B7C'],
    palette: {
      primary: '#A97F7C',
      secondary: '#BFA3A0',
      accent: '#7E8B7C',
      background: '#F0E7E3',
      text: '#2E2524',
    },
  },
  {
    id: 'lavender',
    label: '暗薰衣草',
    description: '低饱和灰紫，静谧柔和',
    dark: true,
    swatches: ['#211E28', '#968AAF', '#6E6A80', '#A3927B'],
    palette: {
      primary: '#968AAF',
      secondary: '#6E6A80',
      accent: '#A3927B',
      background: '#211E28',
      text: '#E4E2EA',
    },
  },
];

export function getPreset(id: ThemePresetId): ThemePreset {
  return THEME_PRESETS.find((preset) => preset.id === id) ?? THEME_PRESETS[0];
}

export function isThemePresetId(id: unknown): id is ThemePresetId {
  return typeof id === 'string' && THEME_PRESETS.some((preset) => preset.id === id);
}
