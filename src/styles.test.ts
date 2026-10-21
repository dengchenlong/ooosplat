import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("keeps topbar controls frameless and changes only color on hover, preserving keyboard focus", () => {
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  const rule = (selector: string) => css.match(new RegExp(`${selector}\\s*\\{([^}]+)\\}`))?.[1];
  expect(rule("\\.settings-action")).toContain("background: transparent");
  expect(rule("\\.settings-action")).toContain("border: 1px solid transparent");
  for (const selector of ["\\.settings-action:hover", "\\.update-notice-action", "\\.update-notice-action:hover"]) {
    const declarations = rule(selector);
    expect(declarations).toContain("color:");
    expect(declarations).not.toMatch(/background|border|box-shadow/);
  }
  expect(rule("button:focus-visible")).toContain("outline: 2px solid");
});

it("rounds only standalone preview action buttons and gives update text an intrinsic-width column", () => {
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  expect(css).toContain('.preview-header-actions > button, .preview-export-menu > button:not(.export-menu-dismiss) { border-radius: 5px; }');
  expect(css.match(/\.preview-header-actions\s*\{[^}]+\}/)?.[0]).toContain("gap: 6px");
  expect(css.match(/\.update-settings-row\s*\{[^}]+\}/)?.[0]).toContain("minmax(0, 1fr) max-content");
  expect(css.match(/\.update-check-button\s*\{[^}]+\}/)?.[0]).toContain("white-space: nowrap");
});

it("mutes readonly media captions and icons without changing new-task icons or reserving filename icon space", () => {
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  const readonlyRule = css.match(/\.source-input-card\.readonly \.media-source-copy > strong,[^}]+\}/)?.[0];
  expect(readonlyRule).toContain(".source-input-card.readonly .media-source-copy > p");
  expect(readonlyRule).toContain(".source-input-card.readonly .media-source-type-icon");
  expect(readonlyRule).toContain("color: #7f8999");
  expect(css.match(/\.media-source-type-icon\s*\{[^}]+\}/)?.[0]).toContain("color: var(--blue)");
  expect(css.match(/\.media-source-heading\s*\{[^}]+\}/)?.[0]).toContain("align-items: center");
  expect(css.match(/\.path-picker\.media-path\s*\{[^}]+\}/)?.[0]).toContain("grid-template-columns: minmax(0, 1fr)");
});
