import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { App } from "./App";
import type { DesktopClient } from "./desktop";

function client(): DesktopClient {
  return {
    bootstrap: vi.fn().mockResolvedValue({
      hardware: {
        cpu: "AMD Ryzen 9 7950X3D",
        memoryBytes: 68_719_476_736,
        gpu: "NVIDIA GeForce RTX 4090",
        vramBytes: 25_769_803_776,
        driver: "591.74",
      },
      settings: {
        schema: 1,
        defaultModel: null,
        installRoot: "C:\\local-models",
        defaultProfile: "stable-16k",
        localMetricsEnabled: true,
      },
      runtime: {
        state: "configured",
        profile: "stable-16k",
        model: "Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
        detail: "The runtime is configured.",
        availableProfiles: ["stable-16k", "turbo-16k"],
      },
    }),
    searchModels: vi.fn().mockResolvedValue([
      {
        id: "Qwen/Qwen3.5-9B-GGUF",
        publisher: "Qwen",
        downloads: 42_000,
        likes: 900,
        lastModified: "2026-08-20T10:00:00.000Z",
        gated: false,
        artifacts: [
          {
            filename: "Qwen3.5-9B-Q4_K_M.gguf",
            sizeBytes: 6_123_456_789,
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            downloadUrl:
              "https://huggingface.co/Qwen/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf",
          },
        ],
      },
    ]),
    assessModel: vi.fn().mockResolvedValue({
      status: "fits-gpu-with-headroom",
      artifactBytes: 6_123_456_789,
      estimatedRuntimeBytes: 7_247_757_312,
      headroomBytes: 18_522_046_464,
      isMeasured: false,
      evidenceLabel: "Estimate — run analysis to measure",
    }),
    setDefaultModel: vi.fn().mockImplementation(async (selection) => ({
      schema: 1,
      defaultModel: selection,
      installRoot: "C:\\local-models",
      defaultProfile: "stable-16k",
      localMetricsEnabled: true,
    })),
    updateSettings: vi.fn().mockImplementation(async (update) => ({ schema: 1, defaultModel: null, ...update })),
    resolvePiLaunch: vi.fn().mockResolvedValue({
      modelId: "Qwen3.5-9B-Q4_K_M.gguf",
      baseUrl: "http://127.0.0.1:8080",
      apiKey: "test-local-token",
      contextWindow: 16_384,
      maxTokens: 2_048,
      temperature: 0.2,
    }),
    runRuntimeProbe: vi.fn().mockResolvedValue({
      model: "Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
      profile: "stable-16k",
      latencyMs: 842,
      outputTokens: 4,
      qualityPass: true,
      evidenceLabel: "Measured diagnostic — not qualification",
    }),
    downloadModel: vi.fn().mockResolvedValue({
      path: "C:\\local-models\\Qwen3.5-9B-Q4_K_M.gguf",
      bytesWritten: 6_123_456_789,
      alreadyPresent: false,
    }),
    cancelDownload: vi.fn().mockResolvedValue(true),
    listDownloads: vi.fn().mockResolvedValue([]),
  };
}

describe("Alpine Desktop primary workflow", () => {
  it("moves from live hardware to a default model without implying qualification", async () => {
    const desktop = client();
    const user = userEvent.setup();
    render(<App desktop={desktop} />);

    expect(await screen.findByText("NVIDIA GeForce RTX 4090")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Models" }));
    await user.type(screen.getByRole("searchbox", { name: "Search Hugging Face" }), "Qwen");
    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.click(await screen.findByRole("button", { name: /Qwen3\.5-9B-GGUF/i }));

    expect(await screen.findByText("Estimate — run analysis to measure")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Set as default" }));
    expect(desktop.setDefaultModel).toHaveBeenCalledWith({
      repoId: "Qwen/Qwen3.5-9B-GGUF",
      filename: "Qwen3.5-9B-Q4_K_M.gguf",
    });
    expect(await screen.findByText("Default for new tasks")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Download model" }));
    expect(desktop.downloadModel).toHaveBeenCalledWith(
      {
        repoId: "Qwen/Qwen3.5-9B-GGUF",
        filename: "Qwen3.5-9B-Q4_K_M.gguf",
      },
      6_123_456_789,
      "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    expect(await screen.findByText("Saved 5.7 GB")).toBeVisible();
  });

  it("opens settings and the browser artifact surface from the task shell", async () => {
    const user = userEvent.setup();
    render(<App desktop={client()} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("heading", { name: "Settings" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Use active model" }));
    expect(screen.getByText("Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Browser" }));
    expect(screen.getByRole("textbox", { name: "Browser address" })).toHaveValue(
      "http://127.0.0.1:4173",
    );
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByTitle("Browser preview")).toHaveAttribute(
      "src",
      "http://127.0.0.1:4173/",
    );
  });

  it("keeps measured analysis disabled when the selected and configured models differ", async () => {
    const user = userEvent.setup();
    render(<App desktop={client()} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    await user.click(screen.getByRole("button", { name: "Analysis" }));

    expect(
      screen.getByText(
        "Select the active runtime model in Settings, or configure the downloaded artifact in Alpine first.",
      ),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Run measured diagnostic" })).toBeDisabled();
  });
});
