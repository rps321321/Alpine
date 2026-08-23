import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface HardwareProfile {
  cpu: string;
  memoryBytes: number;
  gpu: string | null;
  vramBytes: number;
  driver: string | null;
  platform: string;
  architecture: string;
  osVersion: string | null;
  physicalCores: number | null;
  logicalProcessors: number;
  computeDevices: Array<{
    name: string;
    memoryBytes: number;
    driver: string;
    backend: "cuda";
  }>;
}

export interface ModelSelection {
  repoId: string;
  filename: string;
}

export interface DesktopSettings {
  schema: number;
  defaultModel: ModelSelection | null;
  installRoot: string;
  defaultProfile: string;
  localMetricsEnabled: boolean;
  evaluationRepositoryRoot: string;
  browserAllowedHosts: string[];
}

export interface SettingsUpdate {
  installRoot: string;
  defaultProfile: string;
  localMetricsEnabled: boolean;
  evaluationRepositoryRoot: string;
  browserAllowedHosts: string[];
}

export interface BrowserBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface BrowserNavigationRequest {
  tabId: string;
  address: string;
  allowHost: boolean;
  bounds: BrowserBounds;
}

export interface BrowserNavigationResult {
  status: "opened" | "approval-required";
  url: string;
  host: string | null;
}

export type BrowserCommand = "back" | "forward" | "reload" | "focus" | "close";

export type BrowserEvent =
  | { kind: "page"; tabId: string; url: string; loading: boolean }
  | { kind: "title"; tabId: string; title: string }
  | { kind: "accessRequested"; tabId: string; url: string; host: string }
  | { kind: "newTabRequested"; tabId: string; url: string }
  | { kind: "download"; tabId: string; url: string; path: string | null; state: "started" | "completed" | "failed" };

export interface BrowserAdapter {
  readonly nativeSurface: boolean;
  navigate(request: BrowserNavigationRequest): Promise<BrowserNavigationResult>;
  setActive(active: { tabId: string; bounds: BrowserBounds } | null): Promise<void>;
  command(tabId: string, command: BrowserCommand): Promise<void>;
  clearData(): Promise<void>;
  subscribe(listener: (event: BrowserEvent) => void): Promise<() => void>;
}

export interface BootstrapSnapshot {
  hardware: HardwareProfile;
  settings: DesktopSettings;
  runtime: RuntimeSnapshot;
}

export interface RuntimeSnapshot {
  state: "running" | "configured" | "unavailable" | "unconfigured";
  profile: string;
  model: string | null;
  detail: string;
  availableProfiles: string[];
}

export interface PiLaunchConfig {
  modelId: string;
  baseUrl: string;
  apiKey: string;
  contextWindow: number;
  maxTokens: number;
  temperature: number;
}

export interface RuntimeProbeReport {
  model: string;
  profile: string;
  latencyMs: number;
  outputTokens: number | null;
  qualityPass: boolean;
  evidenceLabel: string;
}

export type EvaluationScope = "candidate" | "validated" | "production";

export interface EvaluationProgress {
  state: "running" | "completed" | "failed";
  scope: EvaluationScope;
  message: string;
}

export interface FullEvaluationSummary {
  evaluationId: string;
  scope: EvaluationScope;
  planId: string;
  planSha256: string;
  decision: "qualified" | "unsupported" | "inconclusive" | "regressed" | "not-proven";
  productionDecision: string | null;
  selectedProfile: string | null;
  recommendation: string;
  artifactPath: string;
  tuningMeasurements: Array<{ profile: string; run_id: string }>;
  tuning: Record<string, unknown> | null;
  finalEvidence: Record<string, unknown> | null;
  candidateQualification: Record<string, unknown> | null;
  validatedQualification: Record<string, unknown> | null;
  productionQualification: Record<string, unknown> | null;
  sameProcessRequests: number | null;
  cleanRestarts: number | null;
  nearLimitContextTokens: number | null;
  goldenToolCalls: number | null;
  goldenToolFailures: number | null;
  rollbackProfile: string;
  rollbackProved: boolean;
  priorSessionRestored: boolean;
  deploymentChanged: boolean;
}

export interface DownloadReceipt {
  path: string;
  bytesWritten: number;
  alreadyPresent: boolean;
}

export interface DownloadProgress {
  repoId: string;
  filename: string;
  bytesWritten: number;
  totalBytes: number;
  state: "downloading" | "validating" | "completed" | "cancelled";
}

export interface ModelArtifact {
  filename: string;
  sizeBytes: number;
  sha256: string | null;
  downloadUrl: string;
}

export interface DownloadedModel {
  filename: string;
  sizeBytes: number;
  state: "installed" | "partial";
  source?: "hugging-face" | "import" | null;
  repoId?: string | null;
  revision?: string | null;
  sha256?: string | null;
  localPath?: string;
}

export interface ModelRegistryEntry {
  id: string;
  source: "hugging-face" | "import";
  repoId: string | null;
  revision: string | null;
  filename: string;
  localPath: string;
  observedBytes: number;
  sha256: string;
  originUrl: string | null;
  createdAtMs: number;
  verifiedAtMs: number;
}

export interface DesktopProject {
  id: string;
  name: string;
  root: string;
  createdAtMs: number;
  lastOpenedAtMs: number;
}

export type TaskStatus =
  | "draft"
  | "running"
  | "cancelling"
  | "completed"
  | "cancelled"
  | "failed"
  | "interrupted";

export interface DesktopTask {
  id: string;
  projectId: string;
  title: string;
  status: TaskStatus;
  modelRepoId: string;
  modelFilename: string;
  profile: string;
  error: string | null;
  createdAtMs: number;
  updatedAtMs: number;
}

export interface CreateTaskInput {
  projectId: string;
  title: string;
  modelRepoId: string;
  modelFilename: string;
  profile: string;
}

export type MessageRole = "user" | "assistant" | "system";

export interface TaskMessage {
  id: string;
  taskId: string;
  sequence: number;
  role: MessageRole;
  content: string;
  createdAtMs: number;
}

export interface TaskEvent {
  id: string;
  taskId: string;
  sequence: number;
  kind: string;
  payload: unknown;
  createdAtMs: number;
}

export interface TaskDetail {
  task: DesktopTask;
  messages: TaskMessage[];
  events: TaskEvent[];
}

export type ApprovalState =
  | "pending"
  | "approved"
  | "denied"
  | "executing"
  | "completed"
  | "failed"
  | "interrupted";

export interface ToolApproval {
  id: string;
  taskId: string;
  toolCallId: string;
  operation: "edit" | "shell";
  proposal: Record<string, unknown>;
  state: ApprovalState;
  detail: string | null;
  createdAtMs: number;
  decidedAtMs: number | null;
  settledAtMs: number | null;
}

export interface WorkspaceEntry {
  path: string;
  kind: "file" | "directory";
  sizeBytes: number;
}

export interface WorkspaceRead {
  path: string;
  content: string;
  startLine: number;
  endLine: number;
  totalLines: number;
  truncated: boolean;
}

export interface WorkspaceSearchMatch {
  path: string;
  line: number;
  preview: string;
}

export interface WorkspaceEdit {
  path: string;
  oldText: string;
  newText: string;
}

export interface WorkspaceEditResult {
  path: string;
  replacements: number;
  diff: string;
}

export interface WorkspaceShell {
  command: string;
  timeoutSeconds: number;
}

export interface WorkspaceShellResult {
  command: string;
  exitCode: number;
  stdout: string;
  stderr: string;
  durationMs: number;
  truncated: boolean;
}

export interface ModelSearchResult {
  id: string;
  revision: string | null;
  publisher: string;
  downloads: number;
  likes: number;
  lastModified: string | null;
  gated: boolean;
  artifacts: ModelArtifact[];
}

export interface ModelAssessment {
  status: "fits-gpu-with-headroom" | "fits-gpu-tight" | "fits-with-cpu-offload" | "unlikely-to-fit";
  artifactBytes: number;
  estimatedRuntimeBytes: number;
  headroomBytes: number;
  isMeasured: boolean;
  evidenceLabel: string;
}

export interface PlacementCandidate {
  id: "full-gpu" | "balanced-hybrid" | "conservative-hybrid" | "cpu-only";
  label: string;
  gpuResidencyPercent: number;
  estimatedGpuBytes: number;
  estimatedSystemBytes: number;
  gpuHeadroomBytes: number;
  systemHeadroomBytes: number;
  viable: boolean;
}

export interface PlacementPlan {
  recommendedId: PlacementCandidate["id"] | null;
  candidates: PlacementCandidate[];
  profileHint: string;
  evidenceLabel: string;
}

export interface DesktopClient {
  browser: BrowserAdapter;
  bootstrap(): Promise<BootstrapSnapshot>;
  updateSettings(update: SettingsUpdate): Promise<DesktopSettings>;
  searchModels(query: string): Promise<ModelSearchResult[]>;
  assessModel(artifactBytes: number): Promise<ModelAssessment>;
  planModelPlacement(artifactBytes: number): Promise<PlacementPlan>;
  setDefaultModel(selection: ModelSelection): Promise<DesktopSettings>;
  startRuntime(): Promise<RuntimeSnapshot>;
  stopRuntime(): Promise<RuntimeSnapshot>;
  resolvePiLaunch(): Promise<PiLaunchConfig>;
  runRuntimeProbe(): Promise<RuntimeProbeReport>;
  runFullEvaluation(scope: EvaluationScope): Promise<FullEvaluationSummary>;
  subscribeEvaluationProgress(listener: (progress: EvaluationProgress) => void): Promise<() => void>;
  downloadModel(selection: ModelSelection, revision: string, expectedBytes: number, expectedSha256: string | null): Promise<DownloadReceipt>;
  cancelDownload(selection: ModelSelection): Promise<boolean>;
  listDownloads(): Promise<DownloadedModel[]>;
  importModel(sourcePath: string): Promise<ModelRegistryEntry>;
  listModelRegistry(): Promise<ModelRegistryEntry[]>;
  subscribeDownloadProgress(listener: (progress: DownloadProgress) => void): Promise<() => void>;
  listProjects(): Promise<DesktopProject[]>;
  createProject(name: string, root: string): Promise<DesktopProject>;
  listTasks(projectId: string): Promise<DesktopTask[]>;
  createTask(input: CreateTaskInput): Promise<DesktopTask>;
  loadTask(taskId: string): Promise<TaskDetail | null>;
  appendTaskMessage(input: { taskId: string; role: MessageRole; content: string }): Promise<TaskMessage>;
  appendTaskEvent(input: { taskId: string; kind: string; payload: unknown }): Promise<TaskEvent>;
  setTaskStatus(taskId: string, status: TaskStatus, error?: string | null): Promise<DesktopTask>;
  requestToolApproval(input: {
    taskId: string;
    toolCallId: string;
    operation: "edit" | "shell";
    proposal: Record<string, unknown>;
  }): Promise<ToolApproval>;
  getToolApproval(approvalId: string): Promise<ToolApproval | null>;
  listPendingApprovals(taskId: string): Promise<ToolApproval[]>;
  decideToolApproval(approvalId: string, approved: boolean): Promise<ToolApproval>;
  listProjectFiles(taskId: string, limit?: number): Promise<WorkspaceEntry[]>;
  readProjectFile(taskId: string, path: string, offset?: number, limit?: number): Promise<WorkspaceRead>;
  searchProjectFiles(taskId: string, query: string, limit?: number): Promise<WorkspaceSearchMatch[]>;
  editProjectFile(taskId: string, approvalId: string, edit: WorkspaceEdit): Promise<WorkspaceEditResult>;
  runProjectShell(taskId: string, approvalId: string, shell: WorkspaceShell): Promise<WorkspaceShellResult>;
}

const tauriBrowserAdapter: BrowserAdapter = {
  nativeSurface: true,
  navigate: (request) => invoke<BrowserNavigationResult>("browser_navigate", { request }),
  setActive: (active) => invoke<void>("browser_sync_surface", {
    tabId: active?.tabId ?? null,
    bounds: active?.bounds ?? null,
  }),
  command: (tabId, command) => invoke<void>("browser_command", { tabId, command }),
  clearData: () => invoke<void>("browser_clear_data"),
  subscribe: async (listener) =>
    listen<BrowserEvent>("browser-event", (event) => listener(event.payload)),
};

export const tauriDesktopClient: DesktopClient = {
  browser: tauriBrowserAdapter,
  bootstrap: () => invoke<BootstrapSnapshot>("bootstrap_snapshot"),
  updateSettings: (update) => invoke<DesktopSettings>("update_settings", { update }),
  searchModels: (query) => invoke<ModelSearchResult[]>("search_models", { query }),
  assessModel: (artifactBytes) => invoke<ModelAssessment>("assess_model", { artifactBytes }),
  planModelPlacement: (artifactBytes) =>
    invoke<PlacementPlan>("plan_model_placement", { artifactBytes }),
  setDefaultModel: (selection) =>
    invoke<DesktopSettings>("set_default_model", { selection }),
  startRuntime: () => invoke<RuntimeSnapshot>("start_runtime"),
  stopRuntime: () => invoke<RuntimeSnapshot>("stop_runtime"),
  resolvePiLaunch: () => invoke<PiLaunchConfig>("resolve_pi_launch"),
  runRuntimeProbe: () => invoke<RuntimeProbeReport>("run_runtime_probe"),
  runFullEvaluation: (scope) => invoke<FullEvaluationSummary>("run_full_evaluation", { scope }),
  subscribeEvaluationProgress: async (listener) =>
    listen<EvaluationProgress>("evaluation-progress", (event) => listener(event.payload)),
  downloadModel: (selection, revision, expectedBytes, expectedSha256) =>
    invoke<DownloadReceipt>("download_model", { selection, revision, expectedBytes, expectedSha256 }),
  cancelDownload: (selection) => invoke<boolean>("cancel_download", { selection }),
  listDownloads: () => invoke<DownloadedModel[]>("list_downloads"),
  importModel: (sourcePath) => invoke<ModelRegistryEntry>("import_model", { sourcePath }),
  listModelRegistry: () => invoke<ModelRegistryEntry[]>("list_model_registry"),
  subscribeDownloadProgress: async (listener) =>
    listen<DownloadProgress>("download-progress", (event) => listener(event.payload)),
  listProjects: () => invoke<DesktopProject[]>("list_projects"),
  createProject: (name, root) => invoke<DesktopProject>("create_project", { name, root }),
  listTasks: (projectId) => invoke<DesktopTask[]>("list_tasks", { projectId }),
  createTask: (input) => invoke<DesktopTask>("create_task", { input }),
  loadTask: (taskId) => invoke<TaskDetail | null>("load_task", { taskId }),
  appendTaskMessage: (input) => invoke<TaskMessage>("append_task_message", { input }),
  appendTaskEvent: (input) => invoke<TaskEvent>("append_task_event", { input }),
  setTaskStatus: (taskId, status, error = null) =>
    invoke<DesktopTask>("set_task_status", { taskId, status, error }),
  requestToolApproval: (input) => invoke<ToolApproval>("request_tool_approval", { input }),
  getToolApproval: (approvalId) => invoke<ToolApproval | null>("get_tool_approval", { approvalId }),
  listPendingApprovals: (taskId) => invoke<ToolApproval[]>("list_pending_approvals", { taskId }),
  decideToolApproval: (approvalId, approved) =>
    invoke<ToolApproval>("decide_tool_approval", { approvalId, approved }),
  listProjectFiles: (taskId, limit = 2_000) =>
    invoke<WorkspaceEntry[]>("list_project_files", { taskId, limit }),
  readProjectFile: (taskId, path, offset, limit) =>
    invoke<WorkspaceRead>("read_project_file", { taskId, path, offset, limit }),
  searchProjectFiles: (taskId, query, limit = 200) =>
    invoke<WorkspaceSearchMatch[]>("search_project_files", { taskId, query, limit }),
  editProjectFile: (taskId, approvalId, edit) =>
    invoke<WorkspaceEditResult>("edit_project_file", { taskId, approvalId, edit }),
  runProjectShell: (taskId, approvalId, shell) =>
    invoke<WorkspaceShellResult>("run_project_shell", { taskId, approvalId, shell }),
};

const gib = 1024 ** 3;

const previewNow = Date.now();
const previewProjects: DesktopProject[] = [
  {
    id: "preview-project",
    name: "Alpine",
    root: "C:\\workspace\\Alpine",
    createdAtMs: previewNow,
    lastOpenedAtMs: previewNow,
  },
];
const previewTasks: DesktopTask[] = [];
const previewDetails = new Map<string, TaskDetail>();
const previewApprovals = new Map<string, ToolApproval>();
let previewSequence = 0;

function previewId(prefix: string) {
  previewSequence += 1;
  return `${prefix}-${previewNow}-${previewSequence}`;
}

const previewBrowserListeners = new Set<(event: BrowserEvent) => void>();
const previewBrowserHosts = new Set(["localhost", "127.0.0.1", "::1"]);

function previewBrowserEvent(event: BrowserEvent) {
  for (const listener of previewBrowserListeners) listener(event);
}

const previewBrowserAdapter: BrowserAdapter = {
  nativeSurface: false,
  async navigate(request) {
    const value = /^(https?):\/\//i.test(request.address)
      ? request.address
      : `https://${request.address}`;
    const url = new URL(value);
    if (!request.allowHost && !previewBrowserHosts.has(url.hostname)) {
      return { status: "approval-required", url: url.toString(), host: url.hostname };
    }
    if (request.allowHost) previewBrowserHosts.add(url.hostname);
    queueMicrotask(() => {
      previewBrowserEvent({ kind: "page", tabId: request.tabId, url: url.toString(), loading: true });
      previewBrowserEvent({ kind: "page", tabId: request.tabId, url: url.toString(), loading: false });
    });
    return { status: "opened", url: url.toString(), host: url.hostname };
  },
  async setActive() {},
  async command(tabId, command) {
    if (command === "reload") {
      previewBrowserEvent({ kind: "page", tabId, url: "about:blank", loading: false });
    }
  },
  async clearData() {},
  async subscribe(listener) {
    previewBrowserListeners.add(listener);
    return () => previewBrowserListeners.delete(listener);
  },
};

export const previewDesktopClient: DesktopClient = {
  browser: previewBrowserAdapter,
  async bootstrap() {
    await Promise.resolve();
    return {
      hardware: {
        cpu: "AMD Ryzen 9 7950X3D",
        memoryBytes: 64 * gib,
        gpu: "NVIDIA GeForce RTX 4090",
        vramBytes: 24 * gib,
        driver: "591.74",
        platform: "windows",
        architecture: "x86_64",
        osVersion: "11",
        physicalCores: 16,
        logicalProcessors: 32,
        computeDevices: [{
          name: "NVIDIA GeForce RTX 4090",
          memoryBytes: 24 * gib,
          driver: "591.74",
          backend: "cuda",
        }],
      },
      settings: {
        schema: 3,
        defaultModel: {
          repoId: "local/alpine-install",
          filename: "Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
        },
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
        detail: "The runtime is configured and will start with the next Pi task.",
        availableProfiles: ["stable-16k", "turbo-16k", "fast-32k"],
      },
    };
  },
  async searchModels(query) {
    await new Promise((resolve) => setTimeout(resolve, 250));
    return [
      {
        id: "Qwen/Qwen3.5-9B-GGUF",
        revision: "0123456789abcdef0123456789abcdef01234567",
        publisher: "Qwen",
        downloads: 184_000,
        likes: 2_430,
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
    ].filter((model) => model.id.toLowerCase().includes(query.toLowerCase()));
  },
  async assessModel(artifactBytes) {
    return {
      status: "fits-gpu-with-headroom",
      artifactBytes,
      estimatedRuntimeBytes: Math.ceil((artifactBytes * 1.1 + 512 * 1024 ** 2) / (256 * 1024 ** 2)) * (256 * 1024 ** 2),
      headroomBytes: 17_448_304_640,
      isMeasured: false,
      evidenceLabel: "Estimate — run analysis to measure",
    };
  },
  async planModelPlacement(artifactBytes) {
    const runtimeBytes = Math.ceil((artifactBytes * 1.1 + 512 * 1024 ** 2) / (256 * 1024 ** 2)) * (256 * 1024 ** 2);
    const gpuBudget = 24 * gib * 0.85;
    const systemBudget = 64 * gib * 0.75;
    const candidates: PlacementCandidate[] = [
      ["full-gpu", "Full GPU residency", 100],
      ["balanced-hybrid", "Balanced GPU + CPU", 75],
      ["conservative-hybrid", "Conservative GPU + CPU", 50],
      ["cpu-only", "CPU-only fallback", 0],
    ].map(([id, label, percent]) => {
      const gpuResidencyPercent = Number(percent);
      const estimatedGpuBytes = runtimeBytes * gpuResidencyPercent / 100;
      const estimatedSystemBytes = runtimeBytes - estimatedGpuBytes + (gpuResidencyPercent === 100 ? 512 * 1024 ** 2 : gib);
      return {
        id: id as PlacementCandidate["id"],
        label: String(label),
        gpuResidencyPercent,
        estimatedGpuBytes,
        estimatedSystemBytes,
        gpuHeadroomBytes: Math.max(0, gpuBudget - estimatedGpuBytes),
        systemHeadroomBytes: Math.max(0, systemBudget - estimatedSystemBytes),
        viable: estimatedGpuBytes <= gpuBudget && estimatedSystemBytes <= systemBudget,
      };
    });
    return {
      recommendedId: candidates.find((candidate) => candidate.viable)?.id ?? null,
      candidates,
      profileHint: "Start with the stable Profile; context and layer placement remain measured tuning inputs.",
      evidenceLabel: "Capacity estimate — validate with a bounded Alpine evaluation",
    };
  },
  async setDefaultModel(selection) {
    return {
      schema: 3,
      defaultModel: selection,
      installRoot: "C:\\local-models",
      defaultProfile: "stable-16k",
      evaluationRepositoryRoot: "C:\\workspace\\Alpine",
      localMetricsEnabled: true,
      browserAllowedHosts: [],
    };
  },
  async startRuntime() {
    const snapshot = await this.bootstrap();
    return { ...snapshot.runtime, state: "running", detail: "A verified local llama.cpp session is running." };
  },
  async stopRuntime() {
    const snapshot = await this.bootstrap();
    return { ...snapshot.runtime, state: "configured", detail: "The runtime is configured and stopped." };
  },
  async updateSettings(update) {
    return { schema: 3, defaultModel: null, ...update };
  },
  async resolvePiLaunch() {
    return {
      modelId: "Qwen3.5-9B-Q4_K_M.gguf",
      baseUrl: "http://127.0.0.1:8080",
      apiKey: "preview-local-token",
      contextWindow: 16_384,
      maxTokens: 2_048,
      temperature: 0.2,
    };
  },
  async runRuntimeProbe() {
    await new Promise((resolve) => setTimeout(resolve, 600));
    return {
      model: "Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
      profile: "stable-16k",
      latencyMs: 842,
      outputTokens: 4,
      qualityPass: true,
      evidenceLabel: "Measured diagnostic — not qualification",
    };
  },
  async runFullEvaluation(scope) {
    await new Promise((resolve) => setTimeout(resolve, 700));
    return {
      evaluationId: `preview-${scope}`,
      scope,
      planId: `local-16k-stable-vs-turbo-v1-desktop-${scope}`,
      planSha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
      decision: "qualified",
      productionDecision: scope === "production" ? "not-proven" : null,
      selectedProfile: "turbo-16k",
      recommendation: "Use turbo-16k for this Evidence Identity; no Deployment Role was changed.",
      artifactPath: "C:\\Alpine\\evidence\\evaluations\\preview.json",
      tuningMeasurements: [
        { profile: "stable-16k", run_id: "stable-run" },
        { profile: "turbo-16k", run_id: "turbo-run" },
      ],
      tuning: { disposition: "selected-candidate", reasons: [] },
      finalEvidence: {
        result_summary: {
          workloads: {
            "prefill-4k": { prefill_tps: { median: 722.4 }, quality_pass_rate: 1, deterministic: true },
            "novel-256": { decode_tps: { median: 44.8 }, quality_pass_rate: 1, deterministic: true },
            "repeat-code-256": { decode_tps: { median: 118.6 }, quality_pass_rate: 1, deterministic: true },
            "structured-json-128": { decode_tps: { median: 45.1 }, quality_pass_rate: 1, deterministic: true },
          },
          all_quality_pass: true,
          all_deterministic: true,
          resources: { vram_peak_mib: 22_164, shared_memory_peak_mib: 0 },
        },
      },
      candidateQualification: { decision: "qualified", reasons: [] },
      validatedQualification: scope === "candidate" ? null : { decision: "qualified", reasons: [] },
      productionQualification: null,
      sameProcessRequests: scope === "candidate" ? null : 50,
      cleanRestarts: scope === "candidate" ? null : 10,
      nearLimitContextTokens: scope === "candidate" ? null : 13_926,
      goldenToolCalls: scope === "candidate" ? null : 8,
      goldenToolFailures: scope === "candidate" ? null : 1,
      rollbackProfile: "stable-16k",
      rollbackProved: scope === "production",
      priorSessionRestored: true,
      deploymentChanged: false,
    };
  },
  async subscribeEvaluationProgress() {
    return () => undefined;
  },
  async downloadModel(selection, _revision, expectedBytes) {
    await new Promise((resolve) => setTimeout(resolve, 500));
    return {
      path: `C:\\local-models\\${selection.filename}`,
      bytesWritten: expectedBytes,
      alreadyPresent: false,
    };
  },
  async cancelDownload() {
    return true;
  },
  async listDownloads() {
    return [
      {
        filename: "Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
        sizeBytes: 17_448_304_640,
        state: "installed",
        source: "hugging-face",
        repoId: "Blackfrost-AI/Qwen3.8-27B-ABLITERATED-GGUF",
        revision: "0123456789abcdef0123456789abcdef01234567",
        sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        localPath: "C:\\local-models\\models\\Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
      },
    ];
  },
  async importModel(sourcePath) {
    const filename = sourcePath.split(/[\\/]/).at(-1) ?? "imported.gguf";
    return {
      id: previewId("model"),
      source: "import",
      repoId: null,
      revision: null,
      filename,
      localPath: `C:\\local-models\\models\\${filename}`,
      observedBytes: 6_123_456_789,
      sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      originUrl: null,
      createdAtMs: Date.now(),
      verifiedAtMs: Date.now(),
    };
  },
  async listModelRegistry() {
    return [];
  },
  async subscribeDownloadProgress() {
    return () => undefined;
  },
  async listProjects() {
    return [...previewProjects];
  },
  async createProject(name, root) {
    const project: DesktopProject = {
      id: previewId("project"),
      name,
      root,
      createdAtMs: Date.now(),
      lastOpenedAtMs: Date.now(),
    };
    previewProjects.unshift(project);
    return project;
  },
  async listTasks(projectId) {
    return previewTasks.filter((task) => task.projectId === projectId);
  },
  async createTask(input) {
    const now = Date.now();
    const task: DesktopTask = {
      id: previewId("task"),
      ...input,
      status: "draft",
      error: null,
      createdAtMs: now,
      updatedAtMs: now,
    };
    previewTasks.unshift(task);
    previewDetails.set(task.id, { task, messages: [], events: [] });
    return task;
  },
  async loadTask(taskId) {
    return previewDetails.get(taskId) ?? null;
  },
  async appendTaskMessage(input) {
    const detail = previewDetails.get(input.taskId);
    if (!detail) throw new Error("Preview task does not exist");
    const message: TaskMessage = {
      id: previewId("message"),
      ...input,
      sequence: detail.messages.length + 1,
      createdAtMs: Date.now(),
    };
    detail.messages.push(message);
    return message;
  },
  async appendTaskEvent(input) {
    const detail = previewDetails.get(input.taskId);
    if (!detail) throw new Error("Preview task does not exist");
    const event: TaskEvent = {
      id: previewId("event"),
      ...input,
      sequence: detail.events.length + 1,
      createdAtMs: Date.now(),
    };
    detail.events.push(event);
    return event;
  },
  async setTaskStatus(taskId, status, error = null) {
    const task = previewTasks.find((candidate) => candidate.id === taskId);
    if (!task) throw new Error("Preview task does not exist");
    task.status = status;
    task.error = error;
    task.updatedAtMs = Date.now();
    const detail = previewDetails.get(taskId);
    if (detail) detail.task = task;
    return task;
  },
  async requestToolApproval(input) {
    const approval: ToolApproval = {
      id: previewId("approval"),
      ...input,
      state: "pending",
      detail: null,
      createdAtMs: Date.now(),
      decidedAtMs: null,
      settledAtMs: null,
    };
    previewApprovals.set(approval.id, approval);
    return approval;
  },
  async getToolApproval(approvalId) {
    return previewApprovals.get(approvalId) ?? null;
  },
  async listPendingApprovals(taskId) {
    return [...previewApprovals.values()].filter(
      (approval) => approval.taskId === taskId && approval.state === "pending",
    );
  },
  async decideToolApproval(approvalId, approved) {
    const approval = previewApprovals.get(approvalId);
    if (!approval || approval.state !== "pending") throw new Error("Approval already settled");
    approval.state = approved ? "approved" : "denied";
    approval.decidedAtMs = Date.now();
    return approval;
  },
  async listProjectFiles() {
    return [
      { path: "CONTEXT.md", kind: "file", sizeBytes: 28_412 },
      { path: "apps", kind: "directory", sizeBytes: 0 },
      { path: "apps/desktop/src/App.tsx", kind: "file", sizeBytes: 31_204 },
      { path: "docs", kind: "directory", sizeBytes: 0 },
    ];
  },
  async readProjectFile(_taskId, path) {
    return {
      path,
      content: "# Preview file\n\nThis is project-scoped content from the Alpine Desktop preview.",
      startLine: 1,
      endLine: 3,
      totalLines: 3,
      truncated: false,
    };
  },
  async searchProjectFiles(_taskId, query) {
    return [{ path: "CONTEXT.md", line: 1, preview: `Matched ${query}` }];
  },
  async editProjectFile(_taskId, approvalId, edit) {
    const approval = previewApprovals.get(approvalId);
    if (approval?.state !== "approved") throw new Error("Edit is not approved");
    approval.state = "completed";
    approval.settledAtMs = Date.now();
    return {
      path: edit.path,
      replacements: 1,
      diff: `--- a/${edit.path}\n+++ b/${edit.path}\n-${edit.oldText}\n+${edit.newText}`,
    };
  },
  async runProjectShell(_taskId, approvalId, shell) {
    const approval = previewApprovals.get(approvalId);
    if (approval?.state !== "approved") throw new Error("Shell command is not approved");
    approval.state = "completed";
    approval.settledAtMs = Date.now();
    return {
      command: shell.command,
      exitCode: 0,
      stdout: "All checks passed in preview mode.",
      stderr: "",
      durationMs: 842,
      truncated: false,
    };
  },
};
