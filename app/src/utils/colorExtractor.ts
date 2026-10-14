/**
 * 颜色提取与转换工具
 * 用于从壁纸图片中提取主色调并生成配色方案
 */

export interface RGB {
  r: number; // 0-255
  g: number;
  b: number;
}

export interface HSL {
  h: number; // 0-360
  s: number; // 0-1
  l: number; // 0-1
}

export interface ColorPalette {
  primary: string;      // 主色（最突出的颜色）
  secondary: string;    // 辅助色（色相偏移60度）
  accent: string;       // 强调色（互补色）
  background: string;   // 背景色（深色）
  text: string;         // 文字颜色（自动计算对比度）
}

/**
 * RGB → HSL 转换
 */
export function rgbToHsl(rgb: RGB): HSL {
  const r = rgb.r / 255;
  const g = rgb.g / 255;
  const b = rgb.b / 255;

  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;

  if (max === min) {
    return { h: 0, s: 0, l };
  }

  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);

  let h = 0;
  if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  h /= 6;

  return { h: h * 360, s, l };
}

/**
 * HSL → RGB 转换
 */
export function hslToRgb(hsl: HSL): RGB {
  const { h, s, l } = hsl;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;

  let r = 0,
    g = 0,
    b = 0;
  if (h < 60) {
    r = c;
    g = x;
  } else if (h < 120) {
    r = x;
    g = c;
  } else if (h < 180) {
    g = c;
    b = x;
  } else if (h < 240) {
    g = x;
    b = c;
  } else if (h < 300) {
    r = x;
    b = c;
  } else {
    r = c;
    b = x;
  }

  return {
    r: Math.round((r + m) * 255),
    g: Math.round((g + m) * 255),
    b: Math.round((b + m) * 255),
  };
}

/**
 * RGB → Hex 转换
 */
export function rgbToHex(rgb: RGB): string {
  const toHex = (n: number) => Math.round(n).toString(16).padStart(2, '0');
  return `#${toHex(rgb.r)}${toHex(rgb.g)}${toHex(rgb.b)}`.toUpperCase();
}

/**
 * Hex → RGB 转换
 */
export function hexToRgb(hex: string): RGB {
  const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex);
  return result
    ? {
        r: parseInt(result[1], 16),
        g: parseInt(result[2], 16),
        b: parseInt(result[3], 16),
      }
    : { r: 0, g: 0, b: 0 };
}

/**
 * Hex → RGBA 字符串
 */
export function hexToRgba(hex: string, alpha: number): string {
  const rgb = hexToRgb(hex);
  return `rgba(${rgb.r}, ${rgb.g}, ${rgb.b}, ${alpha})`;
}

/**
 * 计算亮度（用于判断用浅色还是深色文字）
 */
export function getLuminance(hex: string): number {
  const rgb = hexToRgb(hex);
  const r = rgb.r / 255;
  const g = rgb.g / 255;
  const b = rgb.b / 255;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/**
 * 获取饱和度最高的颜色（最鲜艳的颜色）
 */
export function getMostSaturatedColor(samples: RGB[]): string {
  let maxSaturation = 0;
  let mostSaturated = samples[0];

  for (const rgb of samples) {
    const hsl = rgbToHsl(rgb);
    if (hsl.s > maxSaturation && hsl.l > 0.2 && hsl.l < 0.8) {
      // 过滤太暗或太亮的颜色
      maxSaturation = hsl.s;
      mostSaturated = rgb;
    }
  }

  return rgbToHex(mostSaturated);
}

/**
 * 色相偏移
 */
export function shiftHue(hex: string, degrees: number): string {
  const rgb = hexToRgb(hex);
  const hsl = rgbToHsl(rgb);
  hsl.h = (hsl.h + degrees + 360) % 360;
  return rgbToHex(hslToRgb(hsl));
}

/**
 * 互补色（色相偏移180度）
 */
export function getComplementaryColor(hex: string): string {
  return shiftHue(hex, 180);
}

/**
 * 变暗
 */
export function darken(hex: string, amount: number): string {
  const rgb = hexToRgb(hex);
  const hsl = rgbToHsl(rgb);
  hsl.l = Math.max(0, hsl.l - amount);
  return rgbToHex(hslToRgb(hsl));
}

/**
 * 变亮
 */
export function lighten(hex: string, amount: number): string {
  const rgb = hexToRgb(hex);
  const hsl = rgbToHsl(rgb);
  hsl.l = Math.min(1, hsl.l + amount);
  return rgbToHex(hslToRgb(hsl));
}

/**
 * 从图片中提取颜色
 * 使用 Canvas API 采样图片颜色（9宫格采样）
 */
export async function extractColorsFromImage(
  imageFile: File
): Promise<ColorPalette> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    const url = URL.createObjectURL(imageFile);

    img.onload = () => {
      try {
        // 创建 Canvas 采样颜色
        const canvas = document.createElement('canvas');
        const ctx = canvas.getContext('2d');
        if (!ctx) {
          throw new Error('无法创建 Canvas 上下文');
        }

        // 缩小尺寸加快处理
        const maxSize = 400;
        const scale = Math.min(maxSize / img.width, maxSize / img.height);
        canvas.width = img.width * scale;
        canvas.height = img.height * scale;

        ctx.drawImage(img, 0, 0, canvas.width, canvas.height);

        // 9宫格采样
        const samples: RGB[] = [];
        const gridSize = 3;

        for (let y = 0; y < gridSize; y++) {
          for (let x = 0; x < gridSize; x++) {
            const px = Math.floor((x + 0.5) * (canvas.width / gridSize));
            const py = Math.floor((y + 0.5) * (canvas.height / gridSize));

            const data = ctx.getImageData(px, py, 1, 1).data;
            samples.push({ r: data[0], g: data[1], b: data[2] });
          }
        }

        URL.revokeObjectURL(url);

        // 提取主色调（饱和度最高的颜色）
        const primary = getMostSaturatedColor(samples);

        // 生成辅助色（色相偏移60度）
        const secondary = shiftHue(primary, 60);

        // 生成强调色（互补色）
        const accent = getComplementaryColor(primary);

        // 背景色（主色的深色版本）
        const background = darken(primary, 0.7);

        // 文字颜色（根据背景亮度自动选择）
        const text = getLuminance(background) > 0.5 ? '#000000' : '#FFFFFF';

        resolve({ primary, secondary, accent, background, text });
      } catch (error) {
        reject(error);
      }
    };

    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error('图片加载失败'));
    };

    img.src = url;
  });
}

/**
 * 应用配色方案到 CSS 变量
 */
export function applyColorPalette(palette: ColorPalette): void {
  const root = document.documentElement;

  root.style.setProperty('--primary', palette.primary);
  root.style.setProperty('--secondary', palette.secondary);
  root.style.setProperty('--accent', palette.accent);
  root.style.setProperty('--bg-dark', palette.background);
  root.style.setProperty('--text-primary', palette.text);

  // 自动生成辅助色
  root.style.setProperty('--primary-light', lighten(palette.primary, 0.2));
  root.style.setProperty('--primary-dark', darken(palette.primary, 0.2));
  root.style.setProperty('--primary-glow', hexToRgba(palette.primary, 0.5));
}

/**
 * 重置为默认配色
 */
export function resetToDefaultPalette(): void {
  const root = document.documentElement;

  root.style.setProperty('--primary', '#00D9FF');
  root.style.setProperty('--secondary', '#5B9FFF');
  root.style.setProperty('--accent', '#FFB638');
  root.style.setProperty('--bg-dark', '#1a1a2e');
  root.style.setProperty('--text-primary', '#FFFFFF');
  root.style.setProperty('--primary-light', '#33E4FF');
  root.style.setProperty('--primary-dark', '#00A8CC');
  root.style.setProperty('--primary-glow', 'rgba(0, 217, 255, 0.5)');
}

