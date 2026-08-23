import { invoke } from "@tauri-apps/api/core";

export interface HardwareProfile {
  cpu: string;
  memoryBytes: number;
  gpu: string | null;
  vramBytes: number;
  driver: string | null;
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
}

export interface SettingsUpdate {
  installRoot: string;
  defaultProfile: string;
  localMetricsEnabled: boolean;
}

export interface BootstrapSnapshot {
  hardware: HardwareProfile;
  settings: DesktopSettings;
  runtime: {
    state: "running" | "configured" | "unavailable" | "unconfigured";
    profile: string;
    model: string | null;
    detail: string;
    availableProfiles: string[];
  };
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

export interface DownloadReceipt {
  path: string;
  bytesWritten: number;
  alreadyPresent: boolean;
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
}

export interface ModelSearchResult {
  id: string;
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

export interface DesktopClient {
  bootstrap(): Promise<BootstrapSnapshot>;
  updateSettings(update: SettingsUpdate): Promise<DesktopSettings>;
  searchModels(query: string): Promise<ModelSearchResult[]>;
  assessModel(artifactBytes: number): Promise<ModelAssessment>;
  setDefaultModel(selection: ModelSelection): Promise<DesktopSettings>;
  resolvePiLaunch(): Promise<PiLaunchConfig>;
  runRuntimeProbe(): Promise<RuntimeProbeReport>;
  downloadModel(selection: ModelSelection, expectedBytes: number, expectedSha256: string | null): Promise<DownloadReceipt>;
  cancelDownload(selection: ModelSelection): Promise<boolean>;
  listDownloads(): Promise<DownloadedModel[]>;
}

export const tauriDesktopClient: DesktopClient = {
  bootstrap: () => invoke<BootstrapSnapshot>("bootstrap_snapshot"),
  updateSettings: (update) => invoke<DesktopSettings>("update_settings", { update }),
  searchModels: (query) => invoke<ModelSearchResult[]>("search_models", { query }),
  assessModel: (artifactBytes) => invoke<ModelAssessment>("assess_model", { artifactBytes }),
  setDefaultModel: (selection) =>
    invoke<DesktopSettings>("set_default_model", { selection }),
  resolvePiLaunch: () => invoke<PiLaunchConfig>("resolve_pi_launch"),
  runRuntimeProbe: () => invoke<RuntimeProbeReport>("run_runtime_probe"),
  downloadModel: (selection, expectedBytes, expectedSha256) =>
    invoke<DownloadReceipt>("download_model", { selection, expectedBytes, expectedSha256 }),
  cancelDownload: (selection) => invoke<boolean>("cancel_download", { selection }),
  listDownloads: () => invoke<DownloadedModel[]>("list_downloads"),
};

const gib = 1024 ** 3;

export const previewDesktopClient: DesktopClient = {
  async bootstrap() {
    await Promise.resolve();
    return {
      hardware: {
        cpu: "AMD Ryzen 9 7950X3D",
        memoryBytes: 64 * gib,
        gpu: "NVIDIA GeForce RTX 4090",
        vramBytes: 24 * gib,
        driver: "591.74",
      },
      settings: {
        schema: 1,
        defaultModel: {
          repoId: "local/alpine-install",
          filename: "Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf",
        },
        installRoot: "C:\\local-models",
        defaultProfile: "stable-16k",
        localMetricsEnabled: true,
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
  async setDefaultModel(selection) {
    return {
      schema: 1,
      defaultModel: selection,
      installRoot: "C:\\local-models",
      defaultProfile: "stable-16k",
      localMetricsEnabled: true,
    };
  },
  async updateSettings(update) {
    return { schema: 1, defaultModel: null, ...update };
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
  async downloadModel(selection, expectedBytes) {
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
      },
    ];
  },
};
