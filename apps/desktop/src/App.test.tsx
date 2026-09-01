import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import axe from "axe-core";
import { describe, expect, it, vi } from "vitest";
import { App } from "./App";
import type { DesktopClient } from "./desktop";

vi.mock("./harness/pi", () => ({
  PiHarness: class {
    readonly errorMessage = undefined;
    subscribe() {
      return () => undefined;
    }
    async prompt() {
      return undefined;
    }
    abort() {
      return undefined;
    }
    steer() {
      return undefined;
    }
    followUp() {
      return undefined;
    }
  },
}));

function client(): DesktopClient {
  return {
    browser: {
      nativeSurface: false,
      navigate: vi.fn().mockImplementation(async ({ address, allowHost }) => {
        const url = new URL(
          /^https?:\/\//i.test(address) ? address : `https://${address}`,
        );
        if (
          !allowHost &&
          !["localhost", "127.0.0.1", "::1"].includes(url.hostname)
        ) {
          return {
            status: "approval-required" as const,
            url: url.toString(),
            host: url.hostname,
          };
        }
        return {
          status: "opened" as const,
          url: url.toString(),
          host: url.hostname,
        };
      }),
      setActive: vi.fn().mockResolvedValue(undefined),
      command: vi.fn().mockResolvedValue(undefined),
      clearData: vi.fn().mockResolvedValue(undefined),
      subscribe: vi.fn().mockResolvedValue(() => undefined),
    },
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
        computeDevices: [
          {
            name: "NVIDIA GeForce RTX 4090",
            memoryBytes: 25_769_803_776,
            driver: "591.74",
            backend: "cuda" as const,
          },
        ],
      },
      settings: {
        schema: 4,
        defaultModel: null,
        installRoot: "C:\\local-models",
        defaultProfile: "stable-16k",
        localMetricsEnabled: true,
        evaluationRepositoryRoot: "C:\\workspace\\Alpine",
        browserAllowedHosts: [],
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
            sha256:
              "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
      evidenceLabel:
        "Capacity estimate — validate with a bounded Alpine evaluation",
    }),
    setDefaultModel: vi.fn().mockImplementation(async (selection) => ({
      schema: 4,
      defaultModel: selection,
      installRoot: "C:\\local-models",
      defaultProfile: "stable-16k",
      evaluationRepositoryRoot: "C:\\workspace\\Alpine",
      localMetricsEnabled: true,
      browserAllowedHosts: [],
    })),
    updateSettings: vi.fn().mockImplementation(async (update) => ({
      schema: 4,
      defaultModel: null,
      ...update,
    })),
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
      specification: {
        modelRegistryId: "model-1",
        modelRepoId: "Qwen/Qwen3.5-9B-GGUF",
        modelRevision: "a".repeat(40),
        modelFilename: "Qwen3.5-9B-Q4_K_M.gguf",
        modelSha256: "b".repeat(64),
        sessionConfigSha256: "c".repeat(64),
        profileName: "stable-16k",
        profileSha256: "d".repeat(64),
        runtimeName: "official",
        runtimeIdentity: "e".repeat(64),
        adapterIdentity: "pi-agent-core@0.84.2",
        policyIdentity: "alpine-desktop-project-tools-v1",
        contextWindow: 16_384,
        maxTokens: 2_048,
        temperatureMillis: 200,
      },
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
    createExecution: vi.fn().mockImplementation(async ({ taskId, specification }) => ({
      id: "execution-1",
      taskId,
      executionSpecId: "spec-1",
      specification: {
        id: "spec-1",
        taskId,
        ...specification,
        legacyUnverified: false,
        createdAtMs: 1,
      },
      state: "queued",
      failure: null,
      queuedAtMs: 1,
      startedAtMs: null,
      finishedAtMs: null,
      updatedAtMs: 1,
    })),
    transitionExecution: vi.fn().mockImplementation(
      async (executionId, state, failure = null) => ({
        id: executionId,
        taskId: "task-1",
        executionSpecId: "spec-1",
        specification: {
          id: "spec-1",
          taskId: "task-1",
          modelRegistryId: "model-1",
          modelRepoId: "Qwen/Qwen3.5-9B-GGUF",
          modelRevision: "a".repeat(40),
          modelFilename: "Qwen3.5-9B-Q4_K_M.gguf",
          modelSha256: "b".repeat(64),
          sessionConfigSha256: "c".repeat(64),
          profileName: "stable-16k",
          profileSha256: "d".repeat(64),
          runtimeName: "official",
          runtimeIdentity: "e".repeat(64),
          adapterIdentity: "pi-agent-core@0.84.2",
          policyIdentity: "alpine-desktop-project-tools-v1",
          contextWindow: 16_384,
          maxTokens: 2_048,
          temperatureMillis: 200,
          legacyUnverified: false,
          createdAtMs: 1,
        },
        state,
        failure,
        queuedAtMs: 1,
        startedAtMs: 2,
        finishedAtMs: ["completed", "cancelled", "failed", "interrupted"].includes(
          state,
        )
          ? 3
          : null,
        updatedAtMs: 2,
      }),
    ),
    deleteTask: vi.fn().mockResolvedValue(undefined),
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
  it("sends a task with Enter while Shift+Enter keeps a newline", async () => {
    const desktop = client();
    const project = {
      id: "project-1",
      name: "Alpine",
      root: "C:\\workspace\\Alpine",
      createdAtMs: 1,
      lastOpenedAtMs: 2,
    };
    const model = {
      repoId: "Qwen/Qwen3.5-9B-GGUF",
      filename: "Qwen3.5-9B-Q4_K_M.gguf",
    };
    const bootstrap = await desktop.bootstrap();
    const task = {
      id: "task-1",
      projectId: project.id,
      title: "Inspect the app",
      status: "draft" as const,
      modelRepoId: model.repoId,
      modelFilename: model.filename,
      profile: "stable-16k",
      error: null,
      createdAtMs: 3,
      updatedAtMs: 3,
    };
    vi.mocked(desktop.bootstrap).mockResolvedValue({
      ...bootstrap,
      settings: { ...bootstrap.settings, defaultModel: model },
    });
    vi.mocked(desktop.listProjects).mockResolvedValue([project]);
    vi.mocked(desktop.listDownloads).mockResolvedValue([
      {
        ...model,
        sizeBytes: 6_123_456_789,
        state: "installed",
        source: "hugging-face",
        revision: "a".repeat(40),
        sha256: "b".repeat(64),
        localPath: "C:\\models\\qwen.gguf",
      },
    ]);
    vi.mocked(desktop.createTask).mockResolvedValue(task);
    vi.mocked(desktop.setTaskStatus).mockImplementation(
      async (_taskId, status) => ({ ...task, status, updatedAtMs: 4 }),
    );
    vi.mocked(desktop.loadTask).mockResolvedValue({
      task: { ...task, status: "completed" },
      messages: [],
      events: [],
    });
    const user = userEvent.setup();

    render(<App desktop={desktop} />);
    const prompt = await screen.findByRole("textbox", { name: "Task prompt" });
    await user.type(prompt, "Inspect the app{Shift>}{Enter}{/Shift}carefully");
    expect(prompt).toHaveValue("Inspect the app\ncarefully");

    await user.keyboard("{Enter}");

    await waitFor(() =>
      expect(desktop.createTask).toHaveBeenCalledWith(
        expect.objectContaining({ title: "Inspect the app" }),
      ),
    );
    expect(prompt).toHaveValue("");
  });

  it("keeps a failed agent launch visible beside the composer", async () => {
    const desktop = client();
    const project = {
      id: "project-1",
      name: "Alpine",
      root: "C:\\workspace\\Alpine",
      createdAtMs: 1,
      lastOpenedAtMs: 2,
    };
    const model = {
      repoId: "Qwen/Qwen3.5-9B-GGUF",
      filename: "Qwen3.5-9B-Q4_K_M.gguf",
    };
    const bootstrap = await desktop.bootstrap();
    const task = {
      id: "task-1",
      projectId: project.id,
      title: "Inspect the app",
      status: "draft" as const,
      modelRepoId: model.repoId,
      modelFilename: model.filename,
      profile: "stable-16k",
      error: null,
      createdAtMs: 3,
      updatedAtMs: 3,
    };
    vi.mocked(desktop.bootstrap).mockResolvedValue({
      ...bootstrap,
      settings: { ...bootstrap.settings, defaultModel: model },
    });
    vi.mocked(desktop.listProjects).mockResolvedValue([project]);
    vi.mocked(desktop.listDownloads).mockResolvedValue([
      {
        ...model,
        sizeBytes: 6_123_456_789,
        state: "installed",
        source: "hugging-face",
        revision: "a".repeat(40),
        sha256: "b".repeat(64),
        localPath: "C:\\models\\qwen.gguf",
      },
    ]);
    vi.mocked(desktop.createTask).mockResolvedValue(task);
    vi.mocked(desktop.resolvePiLaunch).mockRejectedValue(
      new Error("Local runtime is unavailable"),
    );
    vi.mocked(desktop.setTaskStatus).mockImplementation(
      async (_taskId, status, error) => ({
        ...task,
        status,
        error: error ?? null,
        updatedAtMs: 4,
      }),
    );
    vi.mocked(desktop.loadTask).mockResolvedValue({
      task: {
        ...task,
        status: "failed",
        error: "Local runtime is unavailable",
      },
      messages: [],
      events: [],
    });
    const user = userEvent.setup();

    render(<App desktop={desktop} />);
    const prompt = await screen.findByRole("textbox", { name: "Task prompt" });
    await user.type(prompt, "Inspect the app{Enter}");

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(
      "The local model session is unavailable. Start it in Settings, then try again.",
    );
    await user.click(within(alert).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(desktop.resolvePiLaunch).toHaveBeenCalledTimes(2));
  });

  it("opens Settings with the platform settings shortcut", async () => {
    render(<App desktop={client()} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    fireEvent.keyDown(window, { key: ",", ctrlKey: true });

    expect(screen.getByRole("heading", { name: "Settings" })).toBeVisible();
  });

  it("keeps project management in a collapsible left rail and lets either rail get out of the way", async () => {
    const desktop = client();
    const project = {
      id: "project-1",
      name: "Alpine",
      root: "C:\\workspace\\Alpine",
      createdAtMs: 1,
      lastOpenedAtMs: 2,
    };
    vi.mocked(desktop.listProjects).mockResolvedValue([project]);
    const user = userEvent.setup();

    const first = render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");
    expect(screen.getByText("windows 11 · x86_64")).toBeVisible();
    expect(screen.getByText("16 cores · 32 logical processors")).toBeVisible();
    expect(
      screen.getByText(/Windows chooses interface graphics/),
    ).toBeVisible();

    const projectMenu = await screen.findByRole("button", {
      name: "Alpine project menu",
    });
    await user.click(projectMenu);
    expect(screen.getByRole("menu", { name: "Projects" })).toBeVisible();
    expect(screen.getByRole("menuitem", { name: "Add project" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Hide projects" }));
    expect(
      screen.getByRole("button", { name: "Show projects" }),
    ).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.queryByRole("navigation", { name: "Workspace" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Hide inspector" }));
    expect(
      screen.getByRole("button", { name: "Show inspector" }),
    ).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.queryByRole("complementary", { name: "Context inspector" }),
    ).not.toBeInTheDocument();

    first.unmount();
    render(<App desktop={desktop} />);
    expect(screen.getByRole("button", { name: "Show projects" })).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Show inspector" }),
    ).toBeVisible();
  });

  it("combines discovery and local downloads into one model library", async () => {
    const desktop = client();
    vi.mocked(desktop.listDownloads).mockResolvedValue([
      {
        registryId: "registry-qwen-27b",
        filename: "Qwen3.8-27B-Q4_K_M.gguf",
        sizeBytes: 17_448_304_640,
        state: "installed",
        source: "hugging-face",
        repoId: "Blackfrost/Qwen3.8-27B",
        revision: "a".repeat(40),
        sha256: "b".repeat(64),
        localPath: "C:\\models\\qwen.gguf",
      },
      {
        registryId: "registry-llama-8b",
        filename: "Llama-3.3-8B-Q5_K_M.gguf",
        sizeBytes: 6_123_456_789,
        state: "installed",
        source: "import",
        repoId: null,
        revision: null,
        sha256: "c".repeat(64),
        localPath: "C:\\models\\llama.gguf",
      },
    ]);
    const user = userEvent.setup();

    render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");
    await user.click(screen.getByRole("button", { name: "Models" }));

    expect(
      screen.queryByRole("button", { name: "Downloads" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Model library" }),
    ).toBeVisible();
    expect(screen.getAllByText("Qwen3.8-27B-Q4_K_M.gguf")).not.toHaveLength(0);
    expect(screen.getAllByText("Llama-3.3-8B-Q5_K_M.gguf")).not.toHaveLength(0);

    const selector = screen.getByRole("combobox", {
      name: "Model for new tasks",
    });
    expect(selector).toHaveDisplayValue("Not selected");
    await user.selectOptions(selector, "registry-qwen-27b");
    await waitFor(() =>
      expect(desktop.setDefaultModel).toHaveBeenCalledWith({
        repoId: "Blackfrost/Qwen3.8-27B",
        filename: "Qwen3.8-27B-Q4_K_M.gguf",
        registryId: "registry-qwen-27b",
        revision: "a".repeat(40),
        sha256: "b".repeat(64),
      }),
    );
  });

  it("uses the composer add control and opens the shared browser beside the task", async () => {
    const user = userEvent.setup();
    render(<App desktop={client()} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    await user.click(
      screen.getByRole("button", { name: "Add attachment or tool" }),
    );
    expect(screen.getByRole("menu", { name: "Add to task" })).toBeVisible();
    expect(
      screen.getByRole("menuitem", { name: /Attach image/ }),
    ).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: /Attach PDF/ })).toBeDisabled();
    expect(screen.getByText("Current model is text-only")).toBeVisible();
    expect(
      screen.queryByRole("menuitem", { name: /project/i }),
    ).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(
      screen.queryByRole("menu", { name: "Add to task" }),
    ).not.toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Browser", pressed: false }),
    );
    const address = screen.getByRole("textbox", { name: "Browser address" });
    await user.type(address, "http://127.0.0.1:4173");
    await user.click(screen.getByRole("button", { name: "Go" }));
    expect(screen.getByTitle("Browser page")).toHaveAttribute(
      "src",
      "http://127.0.0.1:4173/",
    );
    expect(
      screen.getByRole("heading", { name: "What should we build?" }),
    ).toBeVisible();
  });

  it("toggles Browser and Performance closed when their active buttons are clicked again", async () => {
    const user = userEvent.setup();
    render(<App desktop={client()} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    const browser = screen.getByRole("button", {
      name: "Browser",
      pressed: false,
    });
    await user.click(browser);
    expect(browser).toHaveAttribute("aria-pressed", "true");
    await user.click(browser);
    expect(browser).toHaveAttribute("aria-pressed", "false");
    expect(
      screen.queryByRole("complementary", { name: "Context inspector" }),
    ).not.toBeInTheDocument();

    const performance = screen.getByRole("button", {
      name: "Performance",
      pressed: false,
    });
    await user.click(performance);
    expect(performance).toHaveAttribute("aria-pressed", "true");
    await user.click(performance);
    expect(performance).toHaveAttribute("aria-pressed", "false");
    expect(
      screen.queryByRole("complementary", { name: "Context inspector" }),
    ).not.toBeInTheDocument();
  });

  it("resizes both side panes with accessible split-view dividers", async () => {
    render(<App desktop={client()} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    const projectsDivider = screen.getByRole("separator", {
      name: "Resize projects",
    });
    expect(projectsDivider).toHaveAttribute("aria-valuenow", "264");
    fireEvent.keyDown(projectsDivider, { key: "ArrowRight" });
    expect(projectsDivider).toHaveAttribute("aria-valuenow", "280");
    expect(window.localStorage.getItem("alpine.ui.leftRailWidth")).toBe("280");

    const contextDivider = screen.getByRole("separator", {
      name: "Resize context panel",
    });
    expect(contextDivider).toHaveAttribute("aria-valuenow", "360");
    fireEvent.keyDown(contextDivider, { key: "ArrowLeft" });
    expect(contextDivider).toHaveAttribute("aria-valuenow", "376");
    expect(window.localStorage.getItem("alpine.ui.inspectorWidth")).toBe("376");
  });

  it("starts with both overlay panes closed at compact desktop widths", async () => {
    const originalWidth = window.innerWidth;
    Object.defineProperty(window, "innerWidth", {
      configurable: true,
      value: 700,
    });
    try {
      render(<App desktop={client()} />);
      await screen.findByRole("heading", { name: "What should we build?" });
      expect(
        screen.getByRole("button", { name: "Show projects" }),
      ).toHaveAttribute("aria-expanded", "false");
      expect(
        screen.getByRole("button", { name: "Show inspector" }),
      ).toHaveAttribute("aria-expanded", "false");
    } finally {
      Object.defineProperty(window, "innerWidth", {
        configurable: true,
        value: originalWidth,
      });
    }
  });

  it("groups tasks by recency and filters the rail immediately", async () => {
    const desktop = client();
    const project = {
      id: "project-1",
      name: "Alpine",
      root: "C:\\workspace\\Alpine",
      createdAtMs: 1,
      lastOpenedAtMs: 2,
    };
    vi.mocked(desktop.listProjects).mockResolvedValue([project]);
    vi.mocked(desktop.listTasks).mockResolvedValue([
      {
        id: "today",
        projectId: project.id,
        title: "Tune Qwen for this PC",
        status: "running",
        modelRepoId: "Qwen/model",
        modelFilename: "qwen.gguf",
        profile: "stable-16k",
        error: null,
        createdAtMs: Date.now(),
        updatedAtMs: Date.now(),
      },
      {
        id: "earlier",
        projectId: project.id,
        title: "Review failed benchmark",
        status: "failed",
        modelRepoId: "Qwen/model",
        modelFilename: "qwen.gguf",
        profile: "stable-16k",
        error: "failed",
        createdAtMs: 1,
        updatedAtMs: 1,
      },
    ]);
    const user = userEvent.setup();

    render(<App desktop={desktop} />);
    expect(await screen.findByText("Tune Qwen for this PC")).toBeVisible();
    expect(screen.getByText("Today")).toBeVisible();
    expect(screen.getByText("Earlier")).toBeVisible();

    const search = screen.getByRole("searchbox", { name: "Search tasks" });
    await user.type(search, "failed");
    expect(screen.queryByText("Tune Qwen for this PC")).not.toBeInTheDocument();
    expect(screen.getByText("Review failed benchmark")).toBeVisible();

    await user.clear(search);
    await user.click(screen.getByRole("button", { name: "Filter tasks" }));
    await user.click(screen.getByRole("menuitem", { name: "Needs attention" }));
    expect(screen.queryByText("Tune Qwen for this PC")).not.toBeInTheDocument();
    expect(screen.getByText("Review failed benchmark")).toBeVisible();
  });

  it("opens the shared browser with Ctrl+Shift+B and asks before a new website", async () => {
    const desktop = client();
    const user = userEvent.setup();
    render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    fireEvent.keyDown(window, { key: "b", ctrlKey: true, shiftKey: true });

    const address = await screen.findByRole("textbox", {
      name: "Browser address",
    });
    expect(
      screen.getByRole("complementary", { name: "Context inspector" }),
    ).toHaveClass("browser-active");
    await user.clear(address);
    await user.type(address, "example.com");
    await user.click(screen.getByRole("button", { name: "Go" }));

    expect(screen.getByText("Allow example.com?")).toBeVisible();
    expect(desktop.browser.navigate).toHaveBeenLastCalledWith(
      expect.objectContaining({
        address: "example.com",
        allowHost: false,
      }),
    );

    await user.click(screen.getByRole("button", { name: "Allow once" }));
    expect(desktop.browser.navigate).toHaveBeenLastCalledWith(
      expect.objectContaining({
        address: "https://example.com/",
        allowHost: true,
      }),
    );
  });

  it("ignores a slower stale model search response", async () => {
    const desktop = client();
    let resolveFirst!: (
      models: Awaited<ReturnType<DesktopClient["searchModels"]>>,
    ) => void;
    vi.mocked(desktop.searchModels)
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolveFirst = resolve;
        }),
      )
      .mockResolvedValueOnce([
        {
          id: "New/Result-GGUF",
          revision: "d".repeat(40),
          publisher: "New",
          downloads: 2,
          likes: 1,
          lastModified: null,
          gated: false,
          artifacts: [],
        },
      ]);
    const user = userEvent.setup();

    render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");
    await user.click(screen.getByRole("button", { name: "Models" }));
    const search = screen.getByRole("searchbox", {
      name: "Search Hugging Face",
    });
    await user.clear(search);
    await user.type(search, "old");
    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.clear(search);
    await user.type(search, "new");
    await user.click(screen.getByRole("button", { name: "Search" }));

    expect(await screen.findByText("New/Result-GGUF")).toBeVisible();
    resolveFirst([
      {
        id: "Old/Result-GGUF",
        revision: "e".repeat(40),
        publisher: "Old",
        downloads: 1,
        likes: 0,
        lastModified: null,
        gated: false,
        artifacts: [],
      },
    ]);
    await waitFor(() =>
      expect(screen.queryByText("Old/Result-GGUF")).not.toBeInTheDocument(),
    );
  });

  it("moves from live hardware to a default model without implying qualification", async () => {
    const desktop = client();
    vi.mocked(desktop.listDownloads)
      .mockResolvedValueOnce([])
      .mockResolvedValue([
        {
          registryId: "registry-qwen-9b",
          filename: "Qwen3.5-9B-Q4_K_M.gguf",
          sizeBytes: 6_123_456_789,
          state: "installed",
          source: "hugging-face",
          repoId: "Qwen/Qwen3.5-9B-GGUF",
          revision: "0123456789abcdef0123456789abcdef01234567",
          sha256:
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          localPath: "C:\\local-models\\models\\Qwen3.5-9B-Q4_K_M.gguf",
        },
      ]);
    const user = userEvent.setup();
    render(<App desktop={desktop} />);

    expect(await screen.findByText("NVIDIA GeForce RTX 4090")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Models" }));
    await user.type(
      screen.getByRole("searchbox", { name: "Search Hugging Face" }),
      "Qwen",
    );
    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.click(
      await screen.findByRole("button", { name: /Qwen3\.5-9B-GGUF/i }),
    );

    expect(
      await screen.findByText("Estimate, not a performance result."),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Download first" }),
    ).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Download" }));
    expect(await screen.findByText("Saved 5.7 GB")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Use for new tasks" }));
    expect(desktop.setDefaultModel).toHaveBeenCalledWith({
      repoId: "Qwen/Qwen3.5-9B-GGUF",
      filename: "Qwen3.5-9B-Q4_K_M.gguf",
      registryId: "registry-qwen-9b",
      revision: "0123456789abcdef0123456789abcdef01234567",
      sha256:
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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

  it("opens browser settings, clears its isolated profile, and returns to the browser surface", async () => {
    const desktop = client();
    vi.mocked(desktop.listDownloads).mockResolvedValue([
      {
        registryId: "registry-active-qwen",
        filename: "Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
        sizeBytes: 17_448_304_640,
        state: "installed",
        source: "hugging-face",
        repoId: "Blackfrost/Qwen3.8-27B-ABLITERATED-GGUF",
        revision: "d".repeat(40),
        sha256: "e".repeat(64),
        localPath:
          "C:\\local-models\\models\\Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
      },
    ]);
    const user = userEvent.setup();
    render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("heading", { name: "Settings" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Use active model" }));
    expect(
      screen.getAllByText("Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf"),
    ).not.toHaveLength(0);

    await user.click(
      within(
        screen.getByRole("navigation", { name: "Settings sections" }),
      ).getByRole("button", { name: "Browser" }),
    );
    expect(screen.getByText("Ask before new websites")).toBeVisible();
    expect(
      screen.getByText("Separate from your regular browser"),
    ).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Clear browsing data" }),
    );
    await waitFor(() =>
      expect(desktop.browser.clearData).toHaveBeenCalledOnce(),
    );
    expect(screen.getByText("Browsing data cleared.")).toBeVisible();

    await user.click(
      screen.getByRole("button", { name: "Browser", pressed: false }),
    );
    expect(
      screen.getByRole("textbox", { name: "Browser address" }),
    ).toBeVisible();
  });

  it("shows verified Pi feature coverage without implying full harness parity", async () => {
    const user = userEvent.setup();
    render(<App desktop={client()} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    await user.click(screen.getByRole("button", { name: "Settings" }));
    await user.click(screen.getByText("Pi feature coverage"));

    expect(screen.getByText("Prompt and streaming")).toBeVisible();
    expect(screen.getByText("Tool approval")).toBeVisible();
    expect(screen.getByText("Harness compaction")).toBeVisible();
    expect(
      screen.getByText(
        "Pi 0.84.2 exposes this in its experimental harness surface, but the shipped implementation is not complete.",
      ),
    ).toBeVisible();
  });

  it("states the execution identity boundary in Safety settings", async () => {
    const user = userEvent.setup();
    render(<App desktop={client()} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    await user.click(screen.getByRole("button", { name: "Settings" }));
    await user.click(
      within(
        screen.getByRole("navigation", { name: "Settings sections" }),
      ).getByRole("button", { name: "Safety" }),
    );

    expect(screen.getByText("Execution identity")).toBeVisible();
    expect(
      screen.getByText(
        "Approved commands run inside the selected project as your current Windows user. Alpine is not a sandbox.",
      ),
    ).toBeVisible();
  });

  it("adds, switches, and closes browser tabs", async () => {
    const desktop = client();
    const user = userEvent.setup();
    render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");
    await user.click(
      screen.getByRole("button", { name: "Browser", pressed: false }),
    );

    await user.click(screen.getByRole("button", { name: "New browser tab" }));
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    await user.click(
      screen.getAllByRole("button", { name: "Close New tab" }).at(-1)!,
    );
    expect(screen.getAllByRole("tab")).toHaveLength(1);
    expect(desktop.browser.command).toHaveBeenCalledWith("browser-2", "close");
  });

  it("provides complete browser history controls", async () => {
    const desktop = client();
    const user = userEvent.setup();
    render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");
    await user.click(
      screen.getByRole("button", { name: "Browser", pressed: false }),
    );
    const address = screen.getByRole("textbox", { name: "Browser address" });
    await user.type(address, "http://127.0.0.1:4173");
    await user.click(screen.getByRole("button", { name: "Go" }));

    await user.click(screen.getByRole("button", { name: "Forward" }));

    expect(desktop.browser.command).toHaveBeenCalledWith(
      "browser-1",
      "forward",
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
    expect(
      screen.getByRole("button", { name: "Run measured diagnostic" }),
    ).toBeDisabled();
  });

  it("keeps one measured analysis active and explains indeterminate progress", async () => {
    const desktop = client();
    const initial = await desktop.bootstrap();
    vi.mocked(desktop.bootstrap).mockResolvedValue({
      ...initial,
      settings: {
        ...initial.settings,
        defaultModel: {
          repoId: "local/alpine-install",
          filename: initial.runtime.model!,
        },
      },
    });
    let finishProbe!: (
      report: Awaited<ReturnType<DesktopClient["runRuntimeProbe"]>>,
    ) => void;
    vi.mocked(desktop.runRuntimeProbe).mockReturnValue(
      new Promise((resolve) => {
        finishProbe = resolve;
      }),
    );
    const user = userEvent.setup();
    render(<App desktop={desktop} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");
    await user.click(screen.getByRole("button", { name: "Analysis" }));

    await user.click(
      screen.getByRole("button", { name: "Run measured diagnostic" }),
    );

    expect(
      screen.getByRole("button", { name: "Run full analysis" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("progressbar", {
        name: "Quick model check in progress",
      }),
    ).toHaveAttribute(
      "aria-valuetext",
      "Starting the local model and measuring an exact response",
    );

    finishProbe({
      model: initial.runtime.model!,
      profile: "stable-16k",
      latencyMs: 842,
      outputTokens: 4,
      qualityPass: true,
      evidenceLabel: "Measured diagnostic — not qualification",
    });
    expect(await screen.findByText("Exact output passed")).toBeVisible();
  });

  it("restores a durable task and renders its pending exact tool approval", async () => {
    const desktop = client();
    const project = {
      id: "project-1",
      name: "Alpine",
      root: "C:\\workspace\\Alpine",
      createdAtMs: 1,
      lastOpenedAtMs: 2,
    };
    const task = {
      id: "task-1",
      projectId: project.id,
      title: "Tighten tests",
      status: "interrupted" as const,
      modelRepoId: "local/alpine-install",
      modelFilename: "model.gguf",
      profile: "stable-16k",
      error: null,
      createdAtMs: 3,
      updatedAtMs: 4,
    };
    vi.mocked(desktop.listProjects).mockResolvedValue([project]);
    vi.mocked(desktop.listTasks).mockResolvedValue([task]);
    vi.mocked(desktop.loadTask).mockResolvedValue({
      task,
      messages: [
        {
          id: "message-1",
          taskId: task.id,
          sequence: 1,
          role: "user",
          content: "Tighten the tests",
          createdAtMs: 3,
        },
      ],
      events: [],
    });
    vi.mocked(desktop.listPendingApprovals).mockResolvedValue([
      {
        id: "approval-1",
        taskId: task.id,
        toolCallId: "tool-1",
        operation: "shell",
        proposal: { command: "npm.cmd test" },
        state: "pending",
        detail: null,
        createdAtMs: 4,
        decidedAtMs: null,
        settledAtMs: null,
      },
    ]);
    const user = userEvent.setup();

    render(<App desktop={desktop} />);
    await user.click(
      await screen.findByRole("button", { name: /Tighten tests/i }),
    );

    expect(await screen.findByText("Tighten the tests")).toBeVisible();
    expect(screen.getByText("Run command?")).toBeVisible();
    expect(screen.getByText(/npm\.cmd test/)).toBeVisible();
  });

  it("runs the bounded evaluation and exposes measured policy evidence", async () => {
    const desktop = client();
    const initial = await desktop.bootstrap();
    vi.mocked(desktop.bootstrap).mockResolvedValue({
      ...initial,
      settings: {
        ...initial.settings,
        defaultModel: {
          repoId: "local/alpine-install",
          filename: initial.runtime.model!,
        },
      },
    });
    vi.mocked(desktop.runFullEvaluation).mockResolvedValue({
      evaluationId: "evaluation-1",
      scope: "candidate",
      planId: "stable-vs-turbo-v1",
      planSha256:
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
      decision: "qualified",
      productionDecision: null,
      selectedProfile: "turbo-16k",
      recommendation:
        "Use turbo-16k for this Evidence Identity; Deployment was not changed.",
      artifactPath: "C:\\evidence\\evaluation-1.json",
      tuningMeasurements: [],
      tuning: {},
      finalEvidence: {
        result_summary: {
          workloads: {},
          all_quality_pass: true,
          all_deterministic: true,
        },
      },
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

    expect(
      await screen.findByRole("heading", { name: "qualified" }),
    ).toBeVisible();
    expect(screen.getByText("Restored")).toBeVisible();
    expect(desktop.runFullEvaluation).toHaveBeenCalledWith("candidate");
  });

  it("has no automated structural WCAG A or AA violations in the primary task shell", async () => {
    const { container } = render(<App desktop={client()} />);
    await screen.findByText("NVIDIA GeForce RTX 4090");

    const result = await axe.run(container, {
      runOnly: {
        type: "tag",
        values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"],
      },
      rules: { "color-contrast": { enabled: false } },
    });

    expect(
      result.violations.map(({ id, nodes }) => ({
        id,
        targets: nodes.map((node) => node.target),
      })),
    ).toEqual([]);
  });
});
