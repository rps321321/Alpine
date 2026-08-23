import { render, screen, waitFor } from "@testing-library/react";
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
        platform: "windows",
        architecture: "x86_64",
        osVersion: "11",
        physicalCores: 16,
        logicalProcessors: 32,
        computeDevices: [{
          name: "NVIDIA GeForce RTX 4090",
          memoryBytes: 25_769_803_776,
          driver: "591.74",
          backend: "cuda" as const,
        }],
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
  it("keeps project management in a collapsible left rail and lets either rail get out of the way", async () => {
    const desktop = client();
    const project = { id: "project-1", name: "Alpine", root: "C:\\workspace\\Alpine", createdAtMs: 1, lastOpenedAtMs: 2 };
    vi.mocked(desktop.listProjects).mockResolvedValue([project]);
    const user = userEvent.setup();

    const first = render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");
    expect(screen.getByText("windows 11 · x86_64")).toBeVisible();
    expect(screen.getByText("16 cores · 32 logical processors")).toBeVisible();
    expect(screen.getByText(/Windows chooses interface graphics/)).toBeVisible();

    const projectMenu = await screen.findByRole("button", { name: "Alpine project menu" });
    await user.click(projectMenu);
    expect(screen.getByRole("menu", { name: "Projects" })).toBeVisible();
    expect(screen.getByRole("menuitem", { name: "Add project" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Hide projects" }));
    expect(screen.getByRole("button", { name: "Show projects" })).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("navigation", { name: "Workspace" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Hide inspector" }));
    expect(screen.getByRole("button", { name: "Show inspector" })).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("complementary", { name: "Context inspector" })).not.toBeInTheDocument();

    first.unmount();
    render(<App desktop={desktop} />);
    expect(screen.getByRole("button", { name: "Show projects" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Show inspector" })).toBeVisible();
  });

  it("combines discovery and local downloads into one model library", async () => {
    const desktop = client();
    vi.mocked(desktop.listDownloads).mockResolvedValue([
      { filename: "Qwen3.8-27B-Q4_K_M.gguf", sizeBytes: 17_448_304_640, state: "installed", source: "hugging-face", repoId: "Blackfrost/Qwen3.8-27B", revision: "a".repeat(40), sha256: "b".repeat(64), localPath: "C:\\models\\qwen.gguf" },
      { filename: "Llama-3.3-8B-Q5_K_M.gguf", sizeBytes: 6_123_456_789, state: "installed", source: "import", repoId: null, revision: null, sha256: "c".repeat(64), localPath: "C:\\models\\llama.gguf" },
    ]);
    const user = userEvent.setup();

    render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");
    await user.click(screen.getByRole("button", { name: "Models" }));

    expect(screen.queryByRole("button", { name: "Downloads" })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Model library" })).toBeVisible();
    expect(screen.getAllByText("Qwen3.8-27B-Q4_K_M.gguf")).not.toHaveLength(0);
    expect(screen.getAllByText("Llama-3.3-8B-Q5_K_M.gguf")).not.toHaveLength(0);

    const selector = screen.getByRole("combobox", { name: "Model for new tasks" });
    expect(selector).toHaveDisplayValue("Not selected");
    await user.selectOptions(selector, "Qwen3.8-27B-Q4_K_M.gguf");
    await waitFor(() => expect(desktop.setDefaultModel).toHaveBeenCalledWith({
      repoId: "Blackfrost/Qwen3.8-27B",
      filename: "Qwen3.8-27B-Q4_K_M.gguf",
    }));
  });

  it("uses the composer add control for attachments and opens previews beside the task", async () => {
    const user = userEvent.setup();
    render(<App desktop={client()} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    await user.click(screen.getByRole("button", { name: "Add attachment or tool" }));
    expect(screen.getByRole("menu", { name: "Add to task" })).toBeVisible();
    expect(screen.getByRole("menuitem", { name: /Attach image/ })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: /Attach PDF/ })).toBeDisabled();
    expect(screen.getByText("Current model is text-only")).toBeVisible();
    expect(screen.queryByRole("menuitem", { name: /project/i })).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu", { name: "Add to task" })).not.toBeInTheDocument();

    expect(screen.queryByRole("button", { name: "Browser" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Preview" }));
    expect(screen.getByRole("textbox", { name: "Preview address" })).toHaveValue("http://127.0.0.1:4173");
    await user.click(screen.getByRole("button", { name: "Open preview" }));
    expect(screen.getByTitle("Browser preview")).toHaveAttribute("src", "http://127.0.0.1:4173/");
    expect(screen.getByRole("heading", { name: "What should we build?" })).toBeVisible();
  });

  it("ignores a slower stale model search response", async () => {
    const desktop = client();
    let resolveFirst!: (models: Awaited<ReturnType<DesktopClient["searchModels"]>>) => void;
    vi.mocked(desktop.searchModels)
      .mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve; }))
      .mockResolvedValueOnce([{ id: "New/Result-GGUF", revision: "d".repeat(40), publisher: "New", downloads: 2, likes: 1, lastModified: null, gated: false, artifacts: [] }]);
    const user = userEvent.setup();

    render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");
    await user.click(screen.getByRole("button", { name: "Models" }));
    const search = screen.getByRole("searchbox", { name: "Search Hugging Face" });
    await user.clear(search);
    await user.type(search, "old");
    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.clear(search);
    await user.type(search, "new");
    await user.click(screen.getByRole("button", { name: "Search" }));

    expect(await screen.findByText("New/Result-GGUF")).toBeVisible();
    resolveFirst([{ id: "Old/Result-GGUF", revision: "e".repeat(40), publisher: "Old", downloads: 1, likes: 0, lastModified: null, gated: false, artifacts: [] }]);
    await waitFor(() => expect(screen.queryByText("Old/Result-GGUF")).not.toBeInTheDocument());
  });

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

    expect(await screen.findByText("Estimate, not a performance result.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Download first" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Download" }));
    expect(await screen.findByText("Saved 5.7 GB")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Use for new tasks" }));
    expect(desktop.setDefaultModel).toHaveBeenCalledWith({
      repoId: "Qwen/Qwen3.5-9B-GGUF",
      filename: "Qwen3.5-9B-Q4_K_M.gguf",
    });
    expect(await screen.findByText("Used for new tasks")).toBeVisible();

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
    expect(screen.getByText("Local addresses only")).toBeVisible();
    expect(screen.getByText("Not enabled")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Preview" }));
    expect(screen.getByRole("textbox", { name: "Preview address" })).toHaveValue(
      "http://127.0.0.1:4173",
    );
    await user.click(screen.getByRole("button", { name: "Open preview" }));
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
        "Choose the same model in Settings that the local runtime is using.",
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
