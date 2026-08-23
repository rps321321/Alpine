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
        schema: 2,
        defaultModel: null,
        installRoot: "C:\\local-models",
        defaultProfile: "stable-16k",
        localMetricsEnabled: true,
        evaluationRepositoryRoot: "C:\\workspace\\Alpine",
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
        revision: "0123456789abcdef0123456789abcdef01234567",
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
    planModelPlacement: vi.fn().mockResolvedValue({
      recommendedId: "full-gpu",
      candidates: [
        {
          id: "full-gpu",
          label: "Full GPU residency",
          gpuResidencyPercent: 100,
          estimatedGpuBytes: 7_247_757_312,
          estimatedSystemBytes: 536_870_912,
          gpuHeadroomBytes: 14_656_000_000,
          systemHeadroomBytes: 50_000_000_000,
          viable: true,
        },
      ],
      profileHint: "Start with the stable Profile.",
      evidenceLabel: "Capacity estimate — validate with a bounded Alpine evaluation",
    }),
    setDefaultModel: vi.fn().mockImplementation(async (selection) => ({
      schema: 2,
      defaultModel: selection,
      installRoot: "C:\\local-models",
      defaultProfile: "stable-16k",
      evaluationRepositoryRoot: "C:\\workspace\\Alpine",
      localMetricsEnabled: true,
    })),
    updateSettings: vi.fn().mockImplementation(async (update) => ({ schema: 2, defaultModel: null, ...update })),
    startRuntime: vi.fn().mockResolvedValue({
      state: "running",
      profile: "stable-16k",
      model: "Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
      detail: "A verified local llama.cpp session is running.",
      availableProfiles: ["stable-16k", "turbo-16k"],
    }),
    stopRuntime: vi.fn().mockResolvedValue({
      state: "configured",
      profile: "stable-16k",
      model: "Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
      detail: "The runtime is configured and stopped.",
      availableProfiles: ["stable-16k", "turbo-16k"],
    }),
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
    runFullEvaluation: vi.fn(),
    subscribeEvaluationProgress: vi.fn().mockResolvedValue(() => undefined),
    downloadModel: vi.fn().mockResolvedValue({
      path: "C:\\local-models\\Qwen3.5-9B-Q4_K_M.gguf",
      bytesWritten: 6_123_456_789,
      alreadyPresent: false,
    }),
    cancelDownload: vi.fn().mockResolvedValue(true),
    listDownloads: vi.fn().mockResolvedValue([]),
    importModel: vi.fn(),
    listModelRegistry: vi.fn().mockResolvedValue([]),
    subscribeDownloadProgress: vi.fn().mockResolvedValue(() => undefined),
    listProjects: vi.fn().mockResolvedValue([]),
    createProject: vi.fn(),
    listTasks: vi.fn().mockResolvedValue([]),
    createTask: vi.fn(),
    loadTask: vi.fn().mockResolvedValue(null),
    appendTaskMessage: vi.fn(),
    appendTaskEvent: vi.fn(),
    setTaskStatus: vi.fn(),
    requestToolApproval: vi.fn(),
    getToolApproval: vi.fn().mockResolvedValue(null),
    listPendingApprovals: vi.fn().mockResolvedValue([]),
    decideToolApproval: vi.fn(),
    listProjectFiles: vi.fn().mockResolvedValue([]),
    readProjectFile: vi.fn(),
    searchProjectFiles: vi.fn().mockResolvedValue([]),
    editProjectFile: vi.fn(),
    runProjectShell: vi.fn(),
  };
}

describe("Alpine Desktop primary workflow", () => {
  it("moves from live hardware to a default model without implying qualification", async () => {
    const desktop = client();
    vi.mocked(desktop.listDownloads)
      .mockResolvedValueOnce([])
      .mockResolvedValue([{ filename: "Qwen3.5-9B-Q4_K_M.gguf", sizeBytes: 6_123_456_789, state: "installed", source: "hugging-face", repoId: "Qwen/Qwen3.5-9B-GGUF", revision: "0123456789abcdef0123456789abcdef01234567", sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", localPath: "C:\\local-models\\models\\Qwen3.5-9B-Q4_K_M.gguf" }]);
    const user = userEvent.setup();
    render(<App desktop={desktop} />);

    expect(await screen.findByText("NVIDIA GeForce RTX 4090")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Models" }));
    await user.type(screen.getByRole("searchbox", { name: "Search Hugging Face" }), "Qwen");
    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.click(await screen.findByRole("button", { name: /Qwen3\.5-9B-GGUF/i }));

    expect(await screen.findByText("Estimate — run analysis to measure")).toBeVisible();
    expect(screen.getByRole("button", { name: "Download before selecting" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Download model" }));
    expect(await screen.findByText("Saved 5.7 GB")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Set as default" }));
    expect(desktop.setDefaultModel).toHaveBeenCalledWith({
      repoId: "Qwen/Qwen3.5-9B-GGUF",
      filename: "Qwen3.5-9B-Q4_K_M.gguf",
    });
    expect(await screen.findByText("Default for new tasks")).toBeVisible();

    expect(desktop.downloadModel).toHaveBeenCalledWith(
      {
        repoId: "Qwen/Qwen3.5-9B-GGUF",
        filename: "Qwen3.5-9B-Q4_K_M.gguf",
      },
      "0123456789abcdef0123456789abcdef01234567",
      6_123_456_789,
      "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
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

  it("restores a durable task and renders its pending exact tool approval", async () => {
    const desktop = client();
    const project = { id: "project-1", name: "Alpine", root: "C:\\workspace\\Alpine", createdAtMs: 1, lastOpenedAtMs: 2 };
    const task = { id: "task-1", projectId: project.id, title: "Tighten tests", status: "interrupted" as const, modelRepoId: "local/alpine-install", modelFilename: "model.gguf", profile: "stable-16k", error: null, createdAtMs: 3, updatedAtMs: 4 };
    vi.mocked(desktop.listProjects).mockResolvedValue([project]);
    vi.mocked(desktop.listTasks).mockResolvedValue([task]);
    vi.mocked(desktop.loadTask).mockResolvedValue({ task, messages: [{ id: "message-1", taskId: task.id, sequence: 1, role: "user", content: "Tighten the tests", createdAtMs: 3 }], events: [] });
    vi.mocked(desktop.listPendingApprovals).mockResolvedValue([{ id: "approval-1", taskId: task.id, toolCallId: "tool-1", operation: "shell", proposal: { command: "npm.cmd test" }, state: "pending", detail: null, createdAtMs: 4, decidedAtMs: null, settledAtMs: null }]);
    const user = userEvent.setup();

    render(<App desktop={desktop} />);
    await user.click(await screen.findByRole("button", { name: /Tighten tests/i }));

    expect(await screen.findByText("Tighten the tests")).toBeVisible();
    expect(screen.getByText("Run command?")).toBeVisible();
    expect(screen.getByText(/npm\.cmd test/)).toBeVisible();
  });

  it("runs the bounded evaluation and exposes measured policy evidence", async () => {
    const desktop = client();
    const initial = await desktop.bootstrap();
    vi.mocked(desktop.bootstrap).mockResolvedValue({
      ...initial,
      settings: { ...initial.settings, defaultModel: { repoId: "local/alpine-install", filename: initial.runtime.model! } },
    });
    vi.mocked(desktop.runFullEvaluation).mockResolvedValue({
      evaluationId: "evaluation-1",
      scope: "candidate",
      planId: "stable-vs-turbo-v1",
      planSha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
      decision: "qualified",
      productionDecision: null,
      selectedProfile: "turbo-16k",
      recommendation: "Use turbo-16k for this Evidence Identity; Deployment was not changed.",
      artifactPath: "C:\\evidence\\evaluation-1.json",
      tuningMeasurements: [],
      tuning: {},
      finalEvidence: { result_summary: { workloads: {}, all_quality_pass: true, all_deterministic: true } },
      candidateQualification: { decision: "qualified" },
      validatedQualification: null,
      productionQualification: null,
      sameProcessRequests: null,
      cleanRestarts: null,
      nearLimitContextTokens: null,
      goldenToolCalls: null,
      goldenToolFailures: null,
      rollbackProfile: "stable-16k",
      rollbackProved: false,
      priorSessionRestored: true,
      deploymentChanged: false,
    });
    const user = userEvent.setup();
    render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");
    await user.click(screen.getByRole("button", { name: "Analysis" }));
    await user.click(screen.getByRole("button", { name: "Run full analysis" }));

    expect(await screen.findByRole("heading", { name: "qualified" })).toBeVisible();
    expect(screen.getByText("Restored")).toBeVisible();
    expect(desktop.runFullEvaluation).toHaveBeenCalledWith("candidate");
  });
});
