import { describe, expect, it } from "vitest";
import "./styles.css";

describe("desktop color tokens", () => {
  it("keeps normal text and status colors at WCAG AA contrast", () => {
    const tokens = getComputedStyle(document.documentElement);
    const backgrounds = [tokens.getPropertyValue("--bg"), tokens.getPropertyValue("--panel")];
    const foregrounds = [
      tokens.getPropertyValue("--text"),
      tokens.getPropertyValue("--muted"),
      tokens.getPropertyValue("--quiet"),
      tokens.getPropertyValue("--good"),
      tokens.getPropertyValue("--warning"),
      tokens.getPropertyValue("--danger"),
    ];

    for (const foreground of foregrounds) {
      for (const background of backgrounds) {
        expect(contrastRatio(foreground, background), `${foreground.trim()} on ${background.trim()}`).toBeGreaterThanOrEqual(4.5);
      }
    }
    expect(contrastRatio(tokens.getPropertyValue("--accent-ink"), tokens.getPropertyValue("--accent"))).toBeGreaterThanOrEqual(4.5);
  });
});

function contrastRatio(foreground: string, background: string) {
  const lighter = Math.max(luminance(foreground), luminance(background));
  const darker = Math.min(luminance(foreground), luminance(background));
  return (lighter + 0.05) / (darker + 0.05);
}

function luminance(value: string) {
  const hex = value.trim().replace(/^#/, "");
  if (!/^[0-9a-f]{6}$/i.test(hex)) throw new Error(`Expected a six-digit hex color, received '${value}'`);
  const channels = [0, 2, 4].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16) / 255);
  return channels.reduce((sum, channel, index) => {
    const linear = channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
    return sum + linear * [0.2126, 0.7152, 0.0722][index];
  }, 0);
}
