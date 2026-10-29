export function invertSelectionShortcutLabel(platform: string) {
  return /^Mac/i.test(platform) ? "⌘+I" : "Ctrl+I";
}

export function isInvertSelectionShortcut(event: KeyboardEvent) {
  if ((!event.ctrlKey && !event.metaKey) || event.altKey || event.shiftKey || event.repeat || event.isComposing) return false;
  if (event.key.toLowerCase() !== "i" && event.code !== "KeyI") return false;
  const editingSelector = "input, textarea, select, [contenteditable]:not([contenteditable='false'])";
  const target = event.target instanceof Element ? event.target : null;
  return !target?.closest(editingSelector) && !document.activeElement?.closest(editingSelector);
}
