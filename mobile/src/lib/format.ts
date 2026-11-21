// @author wzxgqtl 2026-11-21
// 展示用的格式化工具。放在 lib 下，避免路由文件里出现非默认导出。

export function extensionOf(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(dot + 1).toUpperCase() : 'FILE';
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** 只有这些类型在应用内直接渲染，其余交给系统应用打开。 */
export function isInlineText(mime: string): boolean {
  return (
    mime.startsWith('text/') ||
    mime === 'application/json' ||
    mime === 'application/octet-stream'
  );
}
