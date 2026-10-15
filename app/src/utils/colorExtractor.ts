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
  primary: string;      // 主色
  secondary: string;    // 辅助色
  accent: string;       // 强调色
  background: string;   // 背景色
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

function relativeLuminance(hex: string): number {
  const { r, g, b } = hexToRgb(hex);
  const linear = (channel: number) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
}

export function contrastRatio(first: string, second: string): number {
  const a = relativeLuminance(first);
  const b = relativeLuminance(second);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

export function getContrastText(background: string): string {
  const dark = '#17201C';
  const light = '#F3F2EC';
  const preferred = contrastRatio(background, dark) >= contrastRatio(background, light) ? dark : light;
  if (contrastRatio(background, preferred) >= 4.5) return preferred;
  return contrastRatio(background, '#000000') >= contrastRatio(background, '#FFFFFF') ? '#000000' : '#FFFFFF';
}

function readableAccent(color: string, background: string): string {
  const hsl = rgbToHsl(hexToRgb(color));
  const towardLight = getContrastText(background) === '#F3F2EC';
  let result = color;
  for (let step = 0; step < 40 && contrastRatio(result, background) < 4.5; step++) {
    hsl.l = Math.max(0, Math.min(1, hsl.l + (towardLight ? 0.025 : -0.025)));
    result = rgbToHex(hslToRgb(hsl));
  }
  return result;
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
 * 以有界 Canvas 对可见像素分桶，避免透明边缘或少量鲜艳像素主导配色。
 */
export async function extractColorsFromImage(source: Blob | string): Promise<ColorPalette> {
  // The app's CSP deliberately allows data: images, but not blob: URLs.
  // Use the same data URL as the wallpaper so rendering and sampling share a decoder.
  const imageData = typeof source === 'string' ? source : await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error('读取图片数据失败'));
    reader.onabort = () => reject(new Error('图片读取已取消'));
    reader.readAsDataURL(source);
  });
  if (!imageData.startsWith('data:')) throw new Error('取色需要本地图片数据');

  const img = await new Promise<HTMLImageElement>((resolve, reject) => {
    const image = new Image();
    const finish = (error?: Error) => {
      clearTimeout(timeout);
      image.onload = null;
      image.onerror = null;
      if (error) reject(error);
      else resolve(image);
    };
    const timeout = setTimeout(() => finish(new Error('图片解码超时，请重新选择图片')), 12000);
    image.onload = () => finish();
    image.onerror = () => finish(new Error('图片解码失败，请确认图片未损坏且格式受当前 WebView 支持'));
    image.src = imageData;
  });
  if (!img.naturalWidth || !img.naturalHeight) throw new Error('图片尺寸无效');

  const canvas = document.createElement('canvas');
  const scale = Math.min(1, 128 / img.naturalWidth, 128 / img.naturalHeight);
  canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  if (!ctx) throw new Error('无法创建图片取色画布');
  ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
  let pixels: Uint8ClampedArray;
  try {
    pixels = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
  } catch (error) {
    throw new Error(`无法读取图片像素：${error instanceof Error ? error.message : String(error)}`);
  }

  const buckets = new Map<string, { count: number; r: number; g: number; b: number }>();
  const total = { count: 0, r: 0, g: 0, b: 0 };
  for (let i = 0; i < pixels.length; i += 4) {
    if (pixels[i + 3] < 128) continue;
    const r = pixels[i], g = pixels[i + 1], b = pixels[i + 2];
    const key = `${r >> 5},${g >> 5},${b >> 5}`;
    const bucket = buckets.get(key) ?? { count: 0, r: 0, g: 0, b: 0 };
    bucket.count++; bucket.r += r; bucket.g += g; bucket.b += b;
    buckets.set(key, bucket);
    total.count++; total.r += r; total.g += g; total.b += b;
  }
  let dominant: RGB = { r: 131, g: 173, b: 157 };
  let bestScore = -1;
  for (const bucket of buckets.values()) {
    const rgb = { r: bucket.r / bucket.count, g: bucket.g / bucket.count, b: bucket.b / bucket.count };
    const hsl = rgbToHsl(rgb);
    if (hsl.l < 0.12 || hsl.l > 0.9) continue;
    const score = bucket.count * (0.6 + hsl.s);
    if (score > bestScore) { bestScore = score; dominant = rgb; }
  }
  const main = rgbToHsl(dominant);
  main.s = Math.min(main.s, 0.34);
  main.l = Math.max(0.58, Math.min(0.72, main.l));
  const primary = rgbToHex(hslToRgb(main));
  const secondary = rgbToHex(hslToRgb({ ...main, h: (main.h + 30) % 360, s: main.s * 0.65 }));
  const accent = rgbToHex(hslToRgb({ ...main, h: (main.h + 180) % 360, s: Math.min(main.s, 0.25) }));
  const average = total.count ? { r: total.r / total.count, g: total.g / total.count, b: total.b / total.count } : dominant;
  const base = rgbToHsl(average);
  const background = rgbToHex(hslToRgb({ ...base, s: Math.min(base.s, 0.12), l: Math.max(0.065, Math.min(0.11, base.l * 0.18)) }));
  return { primary, secondary, accent, background, text: getContrastText(background) };
}

/**
 * 应用配色方案到 CSS 变量
 */
export function applyColorPalette(palette: ColorPalette): void {
  const root = document.documentElement;
  const lightSurface = getLuminance(palette.background) > 0.52;
  const backgroundHsl = rgbToHsl(hexToRgb(palette.background));
  const surfaceLightness = lightSurface ? Math.max(0.9, Math.min(0.96, backgroundHsl.l + 0.015)) : 0.145;
  const surfaceAt = (lightness: number) => rgbToHex(hslToRgb({ ...backgroundHsl, s: Math.min(backgroundHsl.s, 0.12), l: lightness }));
  const surface = surfaceAt(surfaceLightness);
  const raised = surfaceAt(Math.min(0.985, surfaceLightness + (lightSurface ? 0.025 : 0.035)));
  const input = surfaceAt(surfaceLightness - (lightSurface ? 0.025 : 0.035));
  const border = surfaceAt(surfaceLightness + (lightSurface ? -0.2 : 0.12));
  const subtleBorder = surfaceAt(surfaceLightness + (lightSurface ? -0.12 : 0.075));
  const text = getContrastText(surface);

  root.style.setProperty('--primary', palette.primary);
  root.style.setProperty('--secondary', palette.secondary);
  root.style.setProperty('--accent', palette.accent);
  root.style.setProperty('--bg-dark', palette.background);
  root.style.setProperty('--text-primary', text);
  root.style.setProperty('--text-secondary', hexToRgba(text, 0.9));
  root.style.setProperty('--text-tertiary', hexToRgba(text, 0.8));
  root.style.setProperty(
    '--primary-contrast',
    getContrastText(palette.primary)
  );

  if (lightSurface) {
    root.style.setProperty('color-scheme', 'light');
    root.style.setProperty('--glass-shadow', 'rgba(22, 32, 27, 0.12)');
  } else {
    root.style.setProperty('color-scheme', 'dark');
    root.style.setProperty('--glass-shadow', 'rgba(0, 0, 0, 0.22)');
  }
  root.style.setProperty('--bg-darker', surfaceAt(lightSurface ? surfaceLightness - 0.04 : 0.085));
  root.style.setProperty('--surface', surface);
  root.style.setProperty('--surface-raised', raised);
  root.style.setProperty('--surface-input', input);
  root.style.setProperty('--border', border);
  root.style.setProperty('--border-subtle', subtleBorder);
  root.style.setProperty('--glass-bg', surface);
  root.style.setProperty('--glass-border', border);
  root.style.setProperty('--success', lightSurface ? '#3D7255' : '#98C5A5');
  root.style.setProperty('--warning', lightSurface ? '#80612D' : '#DBBB80');
  root.style.setProperty('--error', lightSurface ? '#A14E48' : '#E2A19A');
  const engineColors = lightSurface
    ? { unity: '#526D40', unreal: '#50688C', rpgmaker: '#925C70', godot: '#3C7180', custom: '#80622D', other: '#5C6860' }
    : { unity: '#9AB783', unreal: '#94A9C3', rpgmaker: '#C58C9E', godot: '#81A9B8', custom: '#D7B77C', other: '#B0BBB5' };
  for (const [engine, color] of Object.entries(engineColors)) root.style.setProperty(`--engine-${engine}`, color);

  // 自动生成辅助色
  root.style.setProperty('--primary-light', readableAccent(palette.primary, surface));
  root.style.setProperty('--primary-dark', darken(palette.primary, 0.08));
  root.style.setProperty('--primary-glow', hexToRgba(palette.primary, 0.18));
}

/**
 * 重置为默认配色
 */
export function resetToDefaultPalette(): void {
  applyColorPalette({ primary: '#83AD9D', secondary: '#9AAFA6', accent: '#D7B77C', background: '#191E1E', text: '#F3F2EC' });
}

