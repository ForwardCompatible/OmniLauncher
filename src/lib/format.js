// Shared formatting helpers for display values. Consolidates the byte/size
// formatters that were previously duplicated across HfFileRow, HfBrowser,
// LocalModelCard, and AppShell.

/**
 * Format a byte count as a human-readable string with one decimal place.
 * @param {number} bytes
 * @returns {string} e.g. "4.1 GB", "256 MB"
 */
export function fmtBytes(bytes) {
  const gb = bytes / 1024 ** 3;
  if (gb >= 1) return gb.toFixed(1) + " GB";
  return (bytes / 1024 ** 2).toFixed(0) + " MB";
}

/**
 * Format a MiB value as a human-readable GB string with one decimal place.
 * Used by the hardware monitor footer (which reports RAM/VRAM in MiB).
 * @param {number} mib
 * @returns {string} e.g. "6.4 GB"
 */
export function fmtMiB(mib) {
  return (mib / 1024).toFixed(1) + " GB";
}
