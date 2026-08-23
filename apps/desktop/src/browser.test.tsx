import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { BrowserSurface } from "./browser";
import type { BrowserAdapter, BrowserEvent } from "./desktop";

describe("shared browser", () => {
  it("turns a requested popup into a new controlled tab and retains host consent", async () => {
    let listener: ((event: BrowserEvent) => void) | undefined;
    const browser: BrowserAdapter = {
      nativeSurface: true,
      navigate: vi.fn().mockResolvedValue({
        status: "approval-required",
        url: "https://accounts.example.com/sign-in",
        host: "accounts.example.com",
      }),
      setActive: vi.fn().mockResolvedValue(undefined),
      command: vi.fn().mockResolvedValue(undefined),
      clearData: vi.fn().mockResolvedValue(undefined),
      subscribe: vi.fn().mockImplementation(async (next) => {
        listener = next;
        return () => undefined;
      }),
    };
    const user = userEvent.setup();
    render(<BrowserSurface browser={browser} />);
    await waitFor(() => expect(listener).toBeDefined());

    act(() => listener?.({
      kind: "newTabRequested",
      tabId: "browser-1",
      url: "https://accounts.example.com/sign-in",
    }));

    expect(await screen.findByText("Allow accounts.example.com?")).toBeVisible();
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    expect(browser.navigate).toHaveBeenCalledWith(expect.objectContaining({
      tabId: "browser-2",
      address: "https://accounts.example.com/sign-in",
      allowHost: false,
    }));

    await user.click(screen.getByRole("button", { name: "Allow once" }));
    expect(browser.navigate).toHaveBeenLastCalledWith(expect.objectContaining({
      tabId: "browser-2",
      address: "https://accounts.example.com/sign-in",
      allowHost: true,
    }));
  });

  it("brings the requesting tab forward before showing a navigation prompt", async () => {
    let listener: ((event: BrowserEvent) => void) | undefined;
    const browser: BrowserAdapter = {
      nativeSurface: true,
      navigate: vi.fn(),
      setActive: vi.fn().mockResolvedValue(undefined),
      command: vi.fn().mockResolvedValue(undefined),
      clearData: vi.fn().mockResolvedValue(undefined),
      subscribe: vi.fn().mockImplementation(async (next) => {
        listener = next;
        return () => undefined;
      }),
    };
    const user = userEvent.setup();
    render(<BrowserSurface browser={browser} />);
    await waitFor(() => expect(listener).toBeDefined());
    await user.click(screen.getByRole("button", { name: "New browser tab" }));
    expect(screen.getAllByRole("tab")[1]).toHaveAttribute("aria-selected", "true");

    act(() => listener?.({
      kind: "accessRequested",
      tabId: "browser-1",
      url: "https://docs.example.com/",
      host: "docs.example.com",
    }));

    expect(screen.getAllByRole("tab")[0]).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("textbox", { name: "Browser address" })).toHaveValue("https://docs.example.com/");
    expect(screen.getByText("Allow docs.example.com?")).toBeVisible();
  });
});
