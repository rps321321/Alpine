import {
  ArrowDown,
  Browser,
  CaretDown,
  CaretRight,
  ChatCircleDots,
  Check,
  CircleNotch,
  Code,
  Cpu,
  DownloadSimple,
  FilePdf,
  FileText,
  Folder,
  FadersHorizontal,
  Gauge,
  GearSix,
  GitDiff,
  HardDrives,
  House,
  Image,
  MagnifyingGlass,
  Paperclip,
  Plus,
  Pulse,
  PuzzlePiece,
  ShieldCheck,
  SidebarSimple,
  Sparkle,
  StopCircle,
  TerminalWindow,
  WarningCircle,
  X,
} from "@phosphor-icons/react";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { BrowserSurface } from "./browser";
import alpineRailMountain from "./assets/alpine-rail-mountain.webp";
import { SplitDivider, useWorkspaceLayout } from "./workspace-layout";
import { PI_RUNTIME_CAPABILITIES } from "./harness/capabilities";
import {
  createTaskExecution,
  type TaskExecution,
  type TaskExecutionUpdate,
} from "./task-execution";
import type {
  BootstrapSnapshot,
  DesktopClient,
  DesktopProject,
  DesktopTask,
  DownloadProgress,
  DownloadedModel,
  EvaluationProgress,
  EvaluationScope,
  FullEvaluationSummary,
  ModelAssessment,
  ModelSearchResult,
  PlacementPlan,
  RuntimeProbeReport,
  SettingsUpdate,
  TaskDetail,
  TaskEvent,
  ToolApproval,
  WorkspaceEntry,
  WorkspaceRead,
} from "./desktop";

type View = "task" | "models" | "analysis" | "settings";
type InspectorTab = "system" | "files" | "changes" | "terminal" | "browser";
type TaskFilter = "all" | "active" | "completed" | "attention";
type TaskRun = {
  taskId?: string;
  prompt: string;
  response: string;
  state: "running" | "cancelling" | "done" | "cancelled" | "error";
  note?: string;
  error?: string;
};

type RendererMetric = { label: string; value: string; detail: string };

const formatBytes = (bytes: number) => {
  if (!bytes) return "Size unavailable";
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
};

const formatCount = (value: number) =>
  new Intl.NumberFormat("en", {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);

function collectRendererMetrics(enabled = true): RendererMetric[] {
  if (!enabled) return [];
  const rows: RendererMetric[] = [];
  const bootstrap = performance
    .getEntriesByName("alpine:bootstrap", "measure")
    .at(-1);
  if (bootstrap)
    rows.push({
      label: "Ready",
      value: `${Math.round(bootstrap.duration)} ms`,
      detail: "renderer start through local bootstrap",
    });
  const piLaunch = performance
    .getEntriesByName("alpine:pi-launch", "measure")
    .at(-1);
  if (piLaunch)
    rows.push({
      label: "Pi launch",
      value: `${Math.round(piLaunch.duration)} ms`,
      detail: "SDK import and local launch resolution",
    });
  const firstEvent = performance
    .getEntriesByName("alpine:stream:first-event", "measure")
    .at(-1);
  if (firstEvent)
    rows.push({
      label: "First stream event",
      value: `${Math.round(firstEvent.duration)} ms`,
      detail: "Pi prompt through first text delta",
    });
  const stream = performance
    .getEntriesByName("alpine:stream:duration", "measure")
    .at(-1);
  if (stream)
    rows.push({
      label: "Stream duration",
      value: `${Math.round(stream.duration)} ms`,
      detail: "complete Pi prompt lifecycle",
    });
  const resources = performance.getEntriesByType(
    "resource",
  ) as PerformanceResourceTiming[];
  const clientBytes = resources
    .filter(
      (entry) =>
        entry.initiatorType === "script" || entry.initiatorType === "link",
    )
    .reduce(
      (total, entry) =>
        total + (entry.transferSize || entry.encodedBodySize || 0),
      0,
    );
  if (clientBytes)
    rows.push({
      label: "Client assets",
      value: formatCompactBytes(clientBytes),
      detail: "current transferred scripts and styles",
    });
  const memory = (
    performance as Performance & { memory?: { usedJSHeapSize: number } }
  ).memory;
  if (memory?.usedJSHeapSize)
    rows.push({
      label: "Renderer heap",
      value: formatCompactBytes(memory.usedJSHeapSize),
      detail: "current JavaScript heap, when exposed by the webview",
    });
  rows.push({
    label: "Long tasks",
    value: String(performance.getEntriesByType("longtask").length),
    detail: "renderer tasks longer than 50 ms",
  });
  return rows;
}

const fitLabel = (assessment: ModelAssessment) => {
  switch (assessment.status) {
    case "fits-gpu-with-headroom":
      return `Fits in graphics memory with ${formatBytes(assessment.headroomBytes)} spare`;
    case "fits-gpu-tight":
      return "Fits in graphics memory, with little room to spare";
    case "fits-with-cpu-offload":
      return "Fits system memory with CPU offload";
    case "unlikely-to-fit":
      return "Unlikely to fit this machine";
  }
};

export function App({ desktop }: { desktop: DesktopClient }) {
  const layout = useWorkspaceLayout();
  const [view, setView] = useState<View>("task");
  const [snapshot, setSnapshot] = useState<BootstrapSnapshot | null>(null);
  const [bootstrapError, setBootstrapError] = useState<string | null>(null);
  const [projects, setProjects] = useState<DesktopProject[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(
    null,
  );
  const [tasks, setTasks] = useState<DesktopTask[]>([]);
  const [taskQuery, setTaskQuery] = useState("");
  const [taskFilter, setTaskFilter] = useState<TaskFilter>("all");
  const [taskFilterOpen, setTaskFilterOpen] = useState(false);
  const [taskDetail, setTaskDetail] = useState<TaskDetail | null>(null);
  const [taskError, setTaskError] = useState<string | null>(null);
  const [showProjectForm, setShowProjectForm] = useState(false);
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);
  const leftRailOpen = layout.left.open;
  const setLeftRailOpen = layout.left.setOpen;
  const inspectorOpen = layout.right.open;
  const setInspectorOpen = layout.right.setOpen;
  const [pendingApprovals, setPendingApprovals] = useState<ToolApproval[]>([]);
  const [workspaceFiles, setWorkspaceFiles] = useState<WorkspaceEntry[]>([]);
  const [workspaceRead, setWorkspaceRead] = useState<WorkspaceRead | null>(
    null,
  );
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>("system");
  const [query, setQuery] = useState("Qwen");
  const [models, setModels] = useState<ModelSearchResult[]>([]);
  const [selected, setSelected] = useState<ModelSearchResult | null>(null);
  const [artifactName, setArtifactName] = useState<string | null>(null);
  const [assessment, setAssessment] = useState<ModelAssessment | null>(null);
  const [placement, setPlacement] = useState<PlacementPlan | null>(null);
  const [searchState, setSearchState] = useState<"idle" | "loading" | "error">(
    "idle",
  );
  const [searchError, setSearchError] = useState<string | null>(null);
  const [defaultSaved, setDefaultSaved] = useState(false);
  const [taskRun, setTaskRun] = useState<TaskRun | null>(null);
  const activeExecution = useRef<TaskExecution | null>(null);
  const workspaceRef = useRef<HTMLElement | null>(null);
  const taskRunLocked = useRef(false);
  const searchRequest = useRef(0);
  const selectionRequest = useRef(0);
  const [downloadState, setDownloadState] = useState<{
    state: "idle" | "running" | "done" | "error";
    message?: string;
  }>({ state: "idle" });
  const [downloadProgress, setDownloadProgress] =
    useState<DownloadProgress | null>(null);
  const [downloads, setDownloads] = useState<DownloadedModel[]>([]);
  const [downloadsError, setDownloadsError] = useState<string | null>(null);
  const [importState, setImportState] = useState<{
    state: "idle" | "running" | "done" | "error";
    message?: string;
  }>({ state: "idle" });
  const [probeState, setProbeState] = useState<{
    state: "idle" | "running" | "done" | "error";
    report?: RuntimeProbeReport;
    error?: string;
  }>({ state: "idle" });
  const [evaluationState, setEvaluationState] = useState<{
    state: "idle" | "running" | "done" | "error";
    scope?: EvaluationScope;
    progress?: EvaluationProgress;
    report?: FullEvaluationSummary;
    error?: string;
  }>({ state: "idle" });
  const [runtimeControl, setRuntimeControl] = useState<{
    state: "idle" | "running" | "error";
    message?: string;
  }>({ state: "idle" });
  const [rendererMetrics, setRendererMetrics] = useState<RendererMetric[]>([]);

  useEffect(() => {
    let active = true;
    Promise.allSettled([
      desktop.bootstrap(),
      desktop.listDownloads(),
      desktop.listProjects(),
    ]).then(([bootstrap, downloaded, knownProjects]) => {
      if (!active) return;
      if (bootstrap.status === "fulfilled") setSnapshot(bootstrap.value);
      else setBootstrapError(errorMessage(bootstrap.reason));
      if (downloaded.status === "fulfilled") setDownloads(downloaded.value);
      else setDownloadsError(errorMessage(downloaded.reason));
      if (knownProjects.status === "fulfilled") {
        setProjects(knownProjects.value);
        setSelectedProjectId(
          (current) => current ?? knownProjects.value[0]?.id ?? null,
        );
      } else {
        setTaskError(errorMessage(knownProjects.reason));
      }
      if (!performance.getEntriesByName("alpine:bootstrap").length) {
        if (!performance.getEntriesByName("alpine:renderer:start").length)
          performance.mark("alpine:renderer:start");
        performance.mark("alpine:bootstrap:ready");
        performance.measure(
          "alpine:bootstrap",
          "alpine:renderer:start",
          "alpine:bootstrap:ready",
        );
      }
      setRendererMetrics(
        collectRendererMetrics(
          bootstrap.status === "fulfilled" &&
            bootstrap.value.settings.localMetricsEnabled,
        ),
      );
    });
    return () => {
      active = false;
    };
  }, [desktop]);

  useEffect(() => {
    if (workspaceRef.current) workspaceRef.current.scrollTop = 0;
  }, [view]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    desktop
      .subscribeEvaluationProgress((progress) =>
        setEvaluationState((current) => ({
          ...current,
          state:
            progress.state === "completed"
              ? "done"
              : progress.state === "failed"
                ? "error"
                : "running",
          scope: progress.scope,
          progress,
          error: progress.state === "failed" ? progress.message : current.error,
        })),
      )
      .then((next) => {
        unlisten = next;
      })
      .catch(() => undefined);
    return () => unlisten?.();
  }, [desktop]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setProjectMenuOpen(false);
        setTaskFilterOpen(false);
        setShowProjectForm(false);
        return;
      }
      if (
        (event.ctrlKey || event.metaKey) &&
        !event.altKey &&
        event.key === ","
      ) {
        event.preventDefault();
        setView("settings");
        setProjectMenuOpen(false);
        return;
      }
      if (
        !event.ctrlKey ||
        event.altKey ||
        event.metaKey ||
        event.key.toLowerCase() !== "b"
      )
        return;
      event.preventDefault();
      if (event.shiftKey) {
        setInspectorOpen((open) => {
          if (open && inspectorTab === "browser") return false;
          setInspectorTab("browser");
          return true;
        });
      } else setLeftRailOpen((open) => !open);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [inspectorTab]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    desktop
      .subscribeDownloadProgress((progress) => setDownloadProgress(progress))
      .then((next) => {
        unlisten = next;
      })
      .catch(() => undefined);
    return () => unlisten?.();
  }, [desktop]);

  useEffect(() => {
    let active = true;
    if (!selectedProjectId) {
      setTasks([]);
      setTaskDetail(null);
      return;
    }
    desktop
      .listTasks(selectedProjectId)
      .then((next) => {
        if (active) setTasks(next);
      })
      .catch((error: unknown) => active && setTaskError(errorMessage(error)));
    return () => {
      active = false;
    };
  }, [desktop, selectedProjectId]);

  const selectedProject =
    projects.find((project) => project.id === selectedProjectId) ?? null;
  const taskGroups = useMemo(() => {
    const query = taskQuery.trim().toLocaleLowerCase();
    const startOfToday = new Date();
    startOfToday.setHours(0, 0, 0, 0);
    const matchesFilter = (task: DesktopTask) => {
      if (taskFilter === "active")
        return task.status === "running" || task.status === "cancelling";
      if (taskFilter === "completed") return task.status === "completed";
      if (taskFilter === "attention")
        return task.status === "failed" || task.status === "interrupted";
      return true;
    };
    const visible = tasks.filter(
      (task) =>
        (!query || task.title.toLocaleLowerCase().includes(query)) &&
        matchesFilter(task),
    );
    return [
      {
        label: "Today",
        tasks: visible.filter(
          (task) => task.updatedAtMs >= startOfToday.getTime(),
        ),
      },
      {
        label: "Earlier",
        tasks: visible.filter(
          (task) => task.updatedAtMs < startOfToday.getTime(),
        ),
      },
    ].filter((group) => group.tasks.length);
  }, [taskFilter, taskQuery, tasks]);
  const selectedArtifact =
    selected?.artifacts.find(
      (artifact) => artifact.filename === artifactName,
    ) ?? selected?.artifacts[0];
  const selectedInstalledModel =
    selected && selectedArtifact && selected.revision
      ? downloads.find(
          (model) =>
            model.state === "installed" &&
            Boolean(model.registryId) &&
            model.repoId === selected.id &&
            model.revision === selected.revision &&
            model.filename === selectedArtifact.filename,
        )
      : undefined;
  const selectedInstalled = Boolean(selectedInstalledModel);
  const hardwareLine = useMemo(() => {
    if (!snapshot) return "Inspecting local hardware…";
    return `${snapshot.hardware.gpu ?? "CPU runtime"} · ${formatBytes(snapshot.hardware.vramBytes || snapshot.hardware.memoryBytes)}`;
  }, [snapshot]);

  const refreshTasks = async (projectId = selectedProjectId) => {
    if (!projectId) return;
    setTasks(await desktop.listTasks(projectId));
  };

  const refreshTask = async (taskId: string) => {
    const [detail, approvals, files] = await Promise.all([
      desktop.loadTask(taskId),
      desktop.listPendingApprovals(taskId),
      desktop.listProjectFiles(taskId).catch(() => []),
    ]);
    setTaskDetail(detail);
    setPendingApprovals(approvals);
    setWorkspaceFiles(files);
    await refreshTasks(detail?.task.projectId);
  };

  const openTask = async (taskId: string) => {
    setView("task");
    setTaskRun(null);
    setTaskError(null);
    try {
      await refreshTask(taskId);
    } catch (error) {
      setTaskError(errorMessage(error));
    }
  };

  const startNewTask = () => {
    setView("task");
    setTaskDetail(null);
    setTaskRun(null);
    setPendingApprovals([]);
    setWorkspaceFiles([]);
    setWorkspaceRead(null);
    setInspectorTab("system");
  };

  const createProject = async (name: string, root: string) => {
    const project = await desktop.createProject(name, root);
    setProjects((current) => [project, ...current]);
    setSelectedProjectId(project.id);
    setShowProjectForm(false);
    startNewTask();
  };

  const ensureTask = async (prompt: string) => {
    if (!snapshot?.settings.defaultModel)
      throw new Error("Choose a default model before creating a Task.");
    if (!selectedProjectId)
      throw new Error("Add a Selected Project before creating a Task.");
    if (taskDetail) {
      return { task: taskDetail.task, history: taskDetail.messages };
    }
    const task = await desktop.createTask({
      projectId: selectedProjectId,
      title: taskTitle(prompt),
      modelRepoId: snapshot.settings.defaultModel.repoId,
      modelFilename: snapshot.settings.defaultModel.filename,
      profile: snapshot.settings.defaultProfile,
    });
    setTaskDetail({ task, messages: [], events: [] });
    await refreshTasks(selectedProjectId);
    return { task, history: [] };
  };

  const runTask = async (prompt: string) => {
    if (taskRunLocked.current) return;
    taskRunLocked.current = true;
    setTaskError(null);
    setTaskRun({ prompt, response: "", state: "running" });
    let taskId: string | undefined;
    try {
      const { task, history } = await ensureTask(prompt);
      taskId = task.id;
      setTaskRun({ taskId, prompt, response: "", state: "running" });
      const execution = createTaskExecution({
        desktop,
        task,
        history,
        measurePerformance: snapshot?.settings.localMetricsEnabled !== false,
        onUpdate: (update) => applyTaskExecutionUpdate(update, taskId!),
      });
      activeExecution.current = execution;
      const result = await execution.run(prompt);
      if (result.state === "error") {
        const message = taskFailureMessage(result.error);
        setTaskError(message);
        setTaskRun({ ...result, error: message });
      } else {
        setTaskRun(result);
      }
    } catch (error) {
      const message = taskFailureMessage(error);
      setTaskError(message);
      setTaskRun({
        taskId,
        prompt,
        response: "",
        state: "error",
        error: message,
      });
    } finally {
      taskRunLocked.current = false;
      activeExecution.current = null;
      if (taskId)
        await refreshTask(taskId).catch((error) =>
          setTaskError(errorMessage(error)),
        );
    }
  };

  const applyTaskExecutionUpdate = (
    update: TaskExecutionUpdate,
    taskId: string,
  ) => {
    switch (update.type) {
      case "response":
        setTaskRun((current) =>
          current?.state === "cancelling"
            ? current
            : { ...update, state: "running" },
        );
        return;
      case "message":
        setTaskDetail((current) =>
          !current || current.task.id !== taskId
            ? current
            : { ...current, messages: [...current.messages, update.message] },
        );
        return;
      case "event":
        setTaskDetail((current) =>
          !current || current.task.id !== taskId
            ? current
            : { ...current, events: [...current.events, update.event] },
        );
        return;
      case "approval":
        setPendingApprovals((current) => [
          ...current.filter((candidate) => candidate.id !== update.approval.id),
          update.approval,
        ]);
        return;
      case "inspector":
        setInspectorTab(update.tab);
        setInspectorOpen(true);
        return;
      case "error":
        setTaskError(
          update.scope === "run"
            ? taskFailureMessage(update.message)
            : update.message,
        );
    }
  };

  const steerTask = (text: string, mode: "steer" | "follow-up") => {
    const execution = activeExecution.current;
    if (!execution) return;
    if (mode === "steer") execution.steer(text);
    else execution.followUp(text);
    setTaskRun((current) =>
      current
        ? {
            ...current,
            note:
              mode === "steer"
                ? "Direction queued for this run"
                : "Follow-up queued",
          }
        : current,
    );
  };

  const cancelTask = () => {
    activeExecution.current?.cancel();
    setTaskRun((current) =>
      current ? { ...current, state: "cancelling", note: undefined } : current,
    );
  };

  const decideApproval = async (approval: ToolApproval, approved: boolean) => {
    const decision = await desktop.decideToolApproval(approval.id, approved);
    setPendingApprovals((current) =>
      current.filter((candidate) => candidate.id !== decision.approval.id),
    );
    setTaskDetail((current) =>
      !current || current.task.id !== decision.event.taskId
        ? current
        : { ...current, events: [...current.events, decision.event] },
    );
  };

  const openWorkspaceFile = async (path: string) => {
    if (!taskDetail) return;
    setInspectorTab("files");
    setWorkspaceRead(await desktop.readProjectFile(taskDetail.task.id, path));
  };

  const search = async (event: FormEvent) => {
    event.preventDefault();
    if (!query.trim()) return;
    const request = ++searchRequest.current;
    setSearchState("loading");
    setSearchError(null);
    setSelected(null);
    setArtifactName(null);
    setAssessment(null);
    setPlacement(null);
    setDefaultSaved(false);
    try {
      const next = await desktop.searchModels(query.trim());
      if (request !== searchRequest.current) return;
      setModels(next);
      setSearchState("idle");
    } catch (error) {
      if (request !== searchRequest.current) return;
      setSearchError(errorMessage(error));
      setSearchState("error");
    }
  };

  const chooseModel = async (model: ModelSearchResult) => {
    const request = ++selectionRequest.current;
    setSelected(model);
    setArtifactName(model.artifacts[0]?.filename ?? null);
    setDefaultSaved(false);
    const artifact = model.artifacts[0];
    if (!artifact?.sizeBytes) {
      setAssessment(null);
      setPlacement(null);
      return;
    }
    try {
      const [nextAssessment, nextPlacement] = await Promise.all([
        desktop.assessModel(artifact.sizeBytes),
        desktop.planModelPlacement(artifact.sizeBytes),
      ]);
      if (request !== selectionRequest.current) return;
      setAssessment(nextAssessment);
      setPlacement(nextPlacement);
    } catch (error) {
      setSearchError(errorMessage(error));
    }
  };

  const chooseArtifact = async (filename: string) => {
    const request = ++selectionRequest.current;
    setArtifactName(filename);
    setDefaultSaved(false);
    const artifact = selected?.artifacts.find(
      (candidate) => candidate.filename === filename,
    );
    if (!artifact?.sizeBytes) {
      setAssessment(null);
      setPlacement(null);
      return;
    }
    const [nextAssessment, nextPlacement] = await Promise.all([
      desktop.assessModel(artifact.sizeBytes),
      desktop.planModelPlacement(artifact.sizeBytes),
    ]);
    if (request !== selectionRequest.current) return;
    setAssessment(nextAssessment);
    setPlacement(nextPlacement);
  };

  const saveDefault = async () => {
    if (!selected || !selectedArtifact || !selectedInstalledModel?.registryId)
      return;
    const settings = await desktop.setDefaultModel({
      repoId: selected.id,
      filename: selectedArtifact.filename,
      registryId: selectedInstalledModel.registryId,
      revision: selectedInstalledModel.revision,
      sha256: selectedInstalledModel.sha256,
    });
    setSnapshot((current) => (current ? { ...current, settings } : current));
    setDefaultSaved(true);
  };

  const selectInstalledModel = async (registryId: string) => {
    const model = downloads.find(
      (candidate) =>
        candidate.state === "installed" && candidate.registryId === registryId,
    );
    if (!model?.registryId || !model.sha256) return;
    const settings = await desktop.setDefaultModel({
      repoId: model.repoId ?? `local/import/${model.sha256}`,
      filename: model.filename,
      registryId: model.registryId,
      revision: model.revision,
      sha256: model.sha256,
    });
    setSnapshot((current) => (current ? { ...current, settings } : current));
    setDefaultSaved(true);
  };

  const downloadSelected = async () => {
    if (!selected || !selectedArtifact) return;
    if (!selected.revision) {
      setDownloadState({
        state: "error",
        message:
          "Hugging Face did not return an exact repository revision; Alpine refused a mutable download.",
      });
      return;
    }
    setDownloadState({ state: "running" });
    try {
      const receipt = await desktop.downloadModel(
        { repoId: selected.id, filename: selectedArtifact.filename },
        selected.revision,
        selectedArtifact.sizeBytes,
        selectedArtifact.sha256,
      );
      setDownloads(await desktop.listDownloads());
      setDownloadState({
        state: "done",
        message: receipt.alreadyPresent
          ? "Already installed"
          : `Saved ${formatBytes(receipt.bytesWritten)}`,
      });
    } catch (error) {
      setDownloadState({ state: "error", message: errorMessage(error) });
    }
  };

  const importModel = async (sourcePath: string) => {
    setImportState({ state: "running" });
    try {
      const model = await desktop.importModel(sourcePath);
      setDownloads(await desktop.listDownloads());
      setImportState({
        state: "done",
        message: `Imported and verified ${model.filename}`,
      });
    } catch (error) {
      setImportState({ state: "error", message: errorMessage(error) });
    }
  };

  const cancelSelectedDownload = async () => {
    if (!selected || !selectedArtifact) return;
    if (
      await desktop.cancelDownload({
        repoId: selected.id,
        filename: selectedArtifact.filename,
      })
    ) {
      setDownloadState({
        state: "running",
        message: "Cancelling after the current chunk…",
      });
    }
  };

  const saveSettings = async (update: SettingsUpdate) => {
    await desktop.updateSettings(update);
    setSnapshot(await desktop.bootstrap());
  };

  const useActiveRuntimeModel = async () => {
    if (!snapshot?.runtime.model) return;
    const matches = downloads.filter(
      (model) =>
        model.state === "installed" &&
        Boolean(model.registryId) &&
        model.filename === snapshot.runtime.model,
    );
    if (matches.length !== 1 || !matches[0].registryId) {
      setRuntimeControl({
        state: "error",
        message:
          matches.length > 1
            ? "More than one verified artifact has this filename. Choose the exact model in the Model Library."
            : "Import or download the active runtime artifact before making it the desktop default.",
      });
      return;
    }
    await selectInstalledModel(matches[0].registryId);
  };

  const runProbe = async () => {
    setProbeState({ state: "running" });
    try {
      const report = await desktop.runRuntimeProbe();
      setProbeState({ state: "done", report });
      setSnapshot(await desktop.bootstrap());
    } catch (error) {
      setProbeState({ state: "error", error: errorMessage(error) });
    }
  };

  const runFullEvaluation = async (scope: EvaluationScope) => {
    setEvaluationState({ state: "running", scope });
    try {
      const report = await desktop.runFullEvaluation(scope);
      setEvaluationState({ state: "done", scope, report });
      setSnapshot(await desktop.bootstrap());
    } catch (error) {
      setEvaluationState({ state: "error", scope, error: errorMessage(error) });
    }
  };

  const controlRuntime = async (action: "start" | "stop") => {
    setRuntimeControl({
      state: "running",
      message:
        action === "start"
          ? "Starting verified session…"
          : "Stopping verified session…",
    });
    try {
      const runtime =
        action === "start"
          ? await desktop.startRuntime()
          : await desktop.stopRuntime();
      setSnapshot((current) => (current ? { ...current, runtime } : current));
      setRuntimeControl({ state: "idle", message: runtime.detail });
    } catch (error) {
      setRuntimeControl({ state: "error", message: errorMessage(error) });
    }
  };

  const navigate = (next: View) => {
    setView(next);
    setProjectMenuOpen(false);
  };

  const toggleInspector = (tab: InspectorTab) => {
    if (inspectorOpen && inspectorTab === tab) {
      setInspectorOpen(false);
      return;
    }
    if (tab === "browser") layout.right.ensureMinimum(480);
    setInspectorTab(tab);
    setInspectorOpen(true);
  };

  return (
    <div
      className={`app-shell ${leftRailOpen ? "" : "left-closed"} ${inspectorOpen ? "" : "right-closed"} ${inspectorOpen && inspectorTab === "browser" ? "browser-open" : ""}`}
      style={layout.style}
    >
      {leftRailOpen && (
        <aside
          className="rail"
          id="project-rail"
          aria-label="Primary navigation"
        >
          <div className="brand">
            <span className="brand-mark">
              <Sparkle size={15} weight="fill" />
            </span>
            <span>Alpine</span>
          </div>
          <button className="new-task" type="button" onClick={startNewTask}>
            <Plus size={16} />
            New task
          </button>
          <nav className="nav-list" aria-label="Workspace">
            <NavButton
              active={view === "task"}
              label="Home"
              onClick={() => navigate("task")}
            >
              <House size={17} />
            </NavButton>
            <NavButton
              active={view === "models"}
              label="Models"
              onClick={() => navigate("models")}
            >
              <HardDrives size={17} />
            </NavButton>
            <NavButton
              active={view === "analysis"}
              label="Analysis"
              onClick={() => navigate("analysis")}
            >
              <Gauge size={17} />
            </NavButton>
          </nav>
          <div className="project-switcher">
            <button
              className="project-menu-trigger"
              type="button"
              aria-haspopup="menu"
              aria-expanded={projectMenuOpen}
              aria-label={`${selectedProject?.name ?? "Choose"} project menu`}
              onClick={() => setProjectMenuOpen((open) => !open)}
            >
              <Folder size={16} />
              <span>
                <small>Project</small>
                {selectedProject?.name ?? "Choose a project"}
              </span>
              <CaretDown size={14} />
            </button>
            {projectMenuOpen && (
              <div className="project-menu" role="menu" aria-label="Projects">
                {projects.map((project) => (
                  <button
                    role="menuitem"
                    type="button"
                    key={project.id}
                    aria-current={
                      project.id === selectedProjectId ? "true" : undefined
                    }
                    onClick={() => {
                      setSelectedProjectId(project.id);
                      setProjectMenuOpen(false);
                      startNewTask();
                    }}
                  >
                    <Folder size={15} />
                    <span>{project.name}</span>
                    {project.id === selectedProjectId && <Check size={14} />}
                  </button>
                ))}
                <button
                  role="menuitem"
                  type="button"
                  onClick={() => {
                    setProjectMenuOpen(false);
                    setShowProjectForm(true);
                  }}
                >
                  <Plus size={15} />
                  <span>Add project</span>
                </button>
              </div>
            )}
          </div>
          <div className="task-tools">
            <label className="task-search">
              <MagnifyingGlass size={14} />
              <input
                type="search"
                aria-label="Search tasks"
                value={taskQuery}
                onChange={(event) => setTaskQuery(event.target.value)}
                placeholder="Search tasks"
              />
            </label>
            <div className="task-filter">
              <button
                type="button"
                aria-label="Filter tasks"
                aria-haspopup="menu"
                aria-expanded={taskFilterOpen}
                className={taskFilter === "all" ? "" : "active"}
                onClick={() => setTaskFilterOpen((open) => !open)}
              >
                <FadersHorizontal size={16} />
              </button>
              {taskFilterOpen && (
                <div
                  className="task-filter-menu"
                  role="menu"
                  aria-label="Task filters"
                >
                  {(
                    [
                      { value: "all", label: "All tasks" },
                      { value: "active", label: "In progress" },
                      { value: "completed", label: "Completed" },
                      { value: "attention", label: "Needs attention" },
                    ] as { value: TaskFilter; label: string }[]
                  ).map((option) => (
                    <button
                      type="button"
                      role="menuitem"
                      key={option.value}
                      aria-current={
                        taskFilter === option.value ? "true" : undefined
                      }
                      onClick={() => {
                        setTaskFilter(option.value);
                        setTaskFilterOpen(false);
                      }}
                    >
                      <span>{option.label}</span>
                      {taskFilter === option.value && <Check size={14} />}
                    </button>
                  ))}
                </div>
              )}
            </div>
          </div>
          <div className="rail-section task-list" aria-label="Tasks">
            {taskGroups.length ? (
              taskGroups.map((group) => (
                <section key={group.label}>
                  <h2>
                    <CaretDown size={12} />
                    {group.label}
                  </h2>
                  {group.tasks.slice(0, 16).map((task) => (
                    <button
                      key={task.id}
                      type="button"
                      className={
                        taskDetail?.task.id === task.id ? "selected" : ""
                      }
                      onClick={() => void openTask(task.id)}
                    >
                      <ChatCircleDots size={14} />
                      <span>{task.title}</span>
                      <small>{friendlyTaskStatus(task.status)}</small>
                    </button>
                  ))}
                </section>
              ))
            ) : (
              <span className="rail-empty">
                {tasks.length
                  ? "No tasks match this filter."
                  : "Your tasks will appear here."}
              </span>
            )}
          </div>
          <div className="rail-spacer" />
          <button
            className="settings-link"
            type="button"
            aria-keyshortcuts="Control+, Meta+,"
            onClick={() => navigate("settings")}
          >
            <GearSix size={17} />
            Settings
          </button>
          <img className="rail-mountain" src={alpineRailMountain} alt="" />
        </aside>
      )}
      {leftRailOpen && (
        <SplitDivider
          label="Resize projects"
          controls="project-rail"
          pane={layout.left}
        />
      )}

      <main className="workspace" ref={workspaceRef}>
        <header className="topbar">
          <div className="topbar-leading">
            <button
              className="panel-toggle"
              type="button"
              aria-label={leftRailOpen ? "Hide projects" : "Show projects"}
              aria-keyshortcuts="Control+B"
              aria-expanded={leftRailOpen}
              onClick={() => setLeftRailOpen((open) => !open)}
            >
              <SidebarSimple size={17} />
            </button>
            <div>
              <strong>
                {taskDetail && view === "task"
                  ? taskDetail.task.title
                  : titleFor(view)}
              </strong>
              <span title={selectedProject?.root}>
                {selectedProject?.name ?? "Choose a project"}
              </span>
            </div>
          </div>
          <div className="top-actions">
            <button
              type="button"
              aria-label="Performance"
              aria-pressed={inspectorOpen && inspectorTab === "system"}
              onClick={() => {
                setRendererMetrics(
                  collectRendererMetrics(
                    snapshot?.settings.localMetricsEnabled !== false,
                  ),
                );
                toggleInspector("system");
              }}
            >
              <Pulse size={16} />
              <span>Performance</span>
            </button>
            <button
              type="button"
              aria-label="Browser"
              aria-keyshortcuts="Control+Shift+B"
              aria-pressed={inspectorOpen && inspectorTab === "browser"}
              onClick={() => toggleInspector("browser")}
            >
              <Browser size={16} />
              <span>Browser</span>
            </button>
            {taskDetail && (
              <span className={`task-state ${taskDetail.task.status}`}>
                {friendlyTaskStatus(taskDetail.task.status)}
              </span>
            )}
            <button
              className="panel-toggle"
              type="button"
              aria-label={inspectorOpen ? "Hide inspector" : "Show inspector"}
              aria-expanded={inspectorOpen}
              onClick={() => setInspectorOpen((open) => !open)}
            >
              <SidebarSimple size={17} />
            </button>
          </div>
        </header>
        {showProjectForm && (
          <ProjectForm
            onClose={() => setShowProjectForm(false)}
            onCreate={createProject}
          />
        )}
        {view === "task" && (
          <TaskView
            snapshot={snapshot}
            bootstrapError={bootstrapError}
            project={selectedProject}
            detail={taskDetail}
            taskError={taskError}
            approvals={pendingApprovals}
            taskRun={taskRun}
            installedModels={downloads}
            onExplore={() => navigate("models")}
            onOpenSettings={() => navigate("settings")}
            onAddProject={() => setShowProjectForm(true)}
            onSelectModel={selectInstalledModel}
            onRun={runTask}
            onQueue={steerTask}
            onCancel={cancelTask}
            onApproval={decideApproval}
          />
        )}
        {view === "models" && (
          <ModelsView
            query={query}
            setQuery={setQuery}
            search={search}
            state={searchState}
            error={searchError}
            models={models}
            selected={selected}
            chooseModel={chooseModel}
            downloads={downloads}
            downloadsError={downloadsError}
            importState={importState}
            onImport={importModel}
            defaultModel={snapshot?.settings.defaultModel ?? null}
            onSelectInstalled={selectInstalledModel}
          />
        )}
        {view === "settings" && (
          <SettingsView
            snapshot={snapshot}
            downloads={downloads}
            runtimeControl={runtimeControl}
            onRuntimeControl={controlRuntime}
            onSave={saveSettings}
            onUseActive={useActiveRuntimeModel}
            onSelectInstalled={selectInstalledModel}
            onClearBrowserData={() => desktop.browser.clearData()}
          />
        )}
        {view === "analysis" && (
          <AnalysisView
            snapshot={snapshot}
            state={probeState}
            evaluation={evaluationState}
            onRun={runProbe}
            onEvaluate={runFullEvaluation}
          />
        )}
      </main>

      {inspectorOpen && (
        <SplitDivider
          label="Resize context panel"
          controls="context-inspector"
          pane={layout.right}
        />
      )}
      {inspectorOpen && (
        <ContextInspector
          browser={desktop.browser}
          tab={inspectorTab}
          setTab={setInspectorTab}
          snapshot={snapshot}
          bootstrapError={bootstrapError}
          selected={view === "models" ? selected : null}
          selectedArtifact={view === "models" ? selectedArtifact : undefined}
          selectedInstalled={view === "models" && selectedInstalled}
          assessment={view === "models" ? assessment : null}
          placement={view === "models" ? placement : null}
          metrics={rendererMetrics}
          defaultSaved={defaultSaved}
          saveDefault={saveDefault}
          chooseArtifact={chooseArtifact}
          downloadState={downloadState}
          downloadProgress={downloadProgress}
          downloadSelected={downloadSelected}
          cancelDownload={cancelSelectedDownload}
          taskDetail={taskDetail}
          files={workspaceFiles}
          workspaceRead={workspaceRead}
          clearWorkspaceRead={() => setWorkspaceRead(null)}
          openFile={openWorkspaceFile}
          hardwareLine={hardwareLine}
        />
      )}
    </div>
  );
}

function NavButton({
  active,
  label,
  onClick,
  children,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      className={active ? "active" : ""}
      aria-current={active ? "page" : undefined}
      onClick={onClick}
    >
      {children}
      <span>{label}</span>
    </button>
  );
}

function ProjectForm({
  onClose,
  onCreate,
}: {
  onClose: () => void;
  onCreate: (name: string, root: string) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [root, setRoot] = useState("");
  const [state, setState] = useState<{ saving: boolean; error?: string }>({
    saving: false,
  });
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setState({ saving: true });
    try {
      await onCreate(name.trim(), root.trim());
    } catch (error) {
      setState({ saving: false, error: errorMessage(error) });
    }
  };
  return (
    <div className="project-form-layer">
      <form className="project-form" onSubmit={submit}>
        <div>
          <span>
            <Folder size={17} />
            Add project
          </span>
          <button
            type="button"
            aria-label="Close project form"
            onClick={onClose}
          >
            <X size={16} />
          </button>
        </div>
        <p>
          Alpine will keep file access and approved changes inside this folder.
        </p>
        <label>
          Project name
          <input
            aria-label="Project name"
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="Alpine"
            required
          />
        </label>
        <label>
          Project folder
          <input
            aria-label="Project folder"
            value={root}
            onChange={(event) => setRoot(event.target.value)}
            placeholder="C:\\workspace\\project"
            required
          />
        </label>
        {state.error && <p className="error-banner">{state.error}</p>}
        <button
          className="primary-button"
          type="submit"
          disabled={state.saving}
        >
          {state.saving ? "Adding…" : "Add project"}
        </button>
      </form>
    </div>
  );
}

function TaskView({
  snapshot,
  bootstrapError,
  project,
  detail,
  taskError,
  approvals,
  taskRun,
  installedModels,
  onExplore,
  onOpenSettings,
  onAddProject,
  onSelectModel,
  onRun,
  onQueue,
  onCancel,
  onApproval,
}: {
  snapshot: BootstrapSnapshot | null;
  bootstrapError: string | null;
  project: DesktopProject | null;
  detail: TaskDetail | null;
  taskError: string | null;
  approvals: ToolApproval[];
  taskRun: TaskRun | null;
  installedModels: DownloadedModel[];
  onExplore: () => void;
  onOpenSettings: () => void;
  onAddProject: () => void;
  onSelectModel: (registryId: string) => Promise<void>;
  onRun: (prompt: string) => Promise<void>;
  onQueue: (prompt: string, mode: "steer" | "follow-up") => void;
  onCancel: () => void;
  onApproval: (approval: ToolApproval, approved: boolean) => Promise<void>;
}) {
  const [draft, setDraft] = useState("");
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  useEffect(() => {
    if (!addMenuOpen) return;
    const closeMenu = (event: KeyboardEvent) => {
      if (event.key === "Escape") setAddMenuOpen(false);
    };
    window.addEventListener("keydown", closeMenu);
    return () => window.removeEventListener("keydown", closeMenu);
  }, [addMenuOpen]);
  const running =
    taskRun?.state === "running" || taskRun?.state === "cancelling";
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const prompt = draft.trim();
    if (!prompt || taskRun?.state === "cancelling") return;
    setDraft("");
    if (running) onQueue(prompt, "steer");
    else void onRun(prompt);
  };
  const messages = detail?.messages ?? [];
  const streamingVisible =
    running &&
    Boolean(taskRun?.response) &&
    messages.at(-1)?.content !== taskRun?.response;
  const activeModel = snapshot?.settings.defaultModel?.filename ?? "";
  const activeModelRegistryId =
    snapshot?.settings.defaultModel?.registryId ??
    installedModels.find(
      (model) =>
        model.registryId &&
        model.filename === snapshot?.settings.defaultModel?.filename &&
        model.repoId === snapshot?.settings.defaultModel?.repoId &&
        (!snapshot?.settings.defaultModel?.revision ||
          model.revision === snapshot.settings.defaultModel.revision),
    )?.registryId ??
    "";
  return (
    <div className="task-view">
      <section className="task-stream" aria-label="Task conversation">
        <div
          className={
            messages.length || taskRun
              ? "task-stream-inner"
              : "task-stream-inner empty-task"
          }
        >
          {messages.length ? (
            <div className="transcript" aria-live="polite">
              {messages.map((message) =>
                message.role === "user" ? (
                  <article className="user-message" key={message.id}>
                    <span>You</span>
                    <p>{message.content}</p>
                  </article>
                ) : (
                  <article className="assistant-message" key={message.id}>
                    <span>
                      <Sparkle size={13} weight="fill" />
                      Alpine
                    </span>
                    <p>{message.content}</p>
                  </article>
                ),
              )}
              {streamingVisible && (
                <article
                  className="assistant-message streaming"
                  aria-label="Alpine is responding"
                >
                  <span>
                    <CircleNotch className="spin" size={13} />
                    Alpine
                  </span>
                  <p>{taskRun?.response}</p>
                </article>
              )}
            </div>
          ) : taskRun ? (
            <div className="transcript" aria-live="polite">
              <article className="user-message">
                <span>You</span>
                <p>{taskRun.prompt}</p>
              </article>
              <article className="assistant-message">
                <span>
                  <CircleNotch className="spin" size={13} />
                  Alpine
                </span>
                {taskRun.state === "error" ? (
                  <div className="error-banner">{taskRun.error}</div>
                ) : (
                  <p>
                    {taskRun.response ||
                      (taskRun.state === "cancelled"
                        ? "Task cancelled."
                        : taskRun.state === "cancelling"
                          ? "Stopping the task…"
                          : "Starting…")}
                  </p>
                )}
              </article>
            </div>
          ) : (
            <div className="empty-task-content">
              <div className="task-kicker">
                <span className="status-dot" />
                {project
                  ? `${project.name} is ready`
                  : "Choose a project to begin"}
              </div>
              <h1>What should we build?</h1>
              <p>
                Ask Alpine to inspect, change, test, or analyze the selected
                project with a local model.
              </p>
              <div className="system-summary">
                <Cpu size={19} />
                <div>
                  <strong>
                    {snapshot?.hardware.cpu ??
                      (bootstrapError
                        ? "Hardware scan unavailable"
                        : "Checking this machine")}
                  </strong>
                  <span>
                    {snapshot
                      ? `${formatBytes(snapshot.hardware.memoryBytes)} memory · ${snapshot.hardware.gpu ?? "CPU only"}`
                      : (bootstrapError ??
                        "Checking CPU, graphics, memory, and local runtime.")}
                  </span>
                </div>
              </div>
              <div className="empty-actions">
                {project ? (
                  <button
                    className="secondary-button explore"
                    type="button"
                    onClick={onExplore}
                  >
                    {activeModel ? "Change model" : "Choose a model"}
                    <ArrowDown size={15} />
                  </button>
                ) : (
                  <button
                    className="primary-button"
                    type="button"
                    onClick={onAddProject}
                  >
                    <Plus size={15} />
                    Add project
                  </button>
                )}
              </div>
            </div>
          )}
        </div>
      </section>
      <div className="task-actions" aria-live="polite">
        {taskError && (
          <div className="inline-task-error" role="alert">
            <WarningCircle size={16} />
            <span>{taskError}</span>
            {taskRun?.state === "error" && taskRun.prompt && (
              <div className="inline-task-error-actions">
                <button
                  className="secondary-button"
                  type="button"
                  onClick={() => void onRun(taskRun.prompt)}
                >
                  Try again
                </button>
                <button type="button" onClick={onOpenSettings}>
                  Open Settings
                </button>
              </div>
            )}
          </div>
        )}
        {approvals.map((approval) => (
          <ApprovalCard
            key={approval.id}
            approval={approval}
            onDecision={onApproval}
          />
        ))}
        {taskRun?.note && (
          <div className="queue-note">
            <Check size={14} />
            {taskRun.note}
          </div>
        )}
      </div>
      <form
        className="composer"
        data-testid="composer"
        aria-label="Task composer"
        onSubmit={submit}
      >
        <textarea
          aria-label="Task prompt"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (
              event.key !== "Enter" ||
              event.shiftKey ||
              event.nativeEvent.isComposing
            )
              return;
            event.preventDefault();
            event.currentTarget.form?.requestSubmit();
          }}
          placeholder={
            running
              ? "Steer the running task…"
              : "Ask Alpine anything about this project…"
          }
        />
        <div className="composer-footer">
          <div className="composer-context">
            <div className="add-menu-wrap">
              <button
                className="composer-icon"
                type="button"
                aria-label="Add attachment or tool"
                aria-haspopup="menu"
                aria-expanded={addMenuOpen}
                onClick={() => setAddMenuOpen((open) => !open)}
              >
                <Plus size={18} />
              </button>
              {addMenuOpen && (
                <div className="add-menu" role="menu" aria-label="Add to task">
                  <button
                    role="menuitem"
                    type="button"
                    disabled
                    title="Available when the selected local model supports images."
                  >
                    <Image size={17} />
                    <span>
                      Attach image<small>Current model is text-only</small>
                    </span>
                  </button>
                  <button
                    role="menuitem"
                    type="button"
                    disabled
                    title="PDF extraction is not installed in this build."
                  >
                    <FilePdf size={17} />
                    <span>
                      Attach PDF<small>PDF support not installed</small>
                    </span>
                  </button>
                  <button
                    role="menuitem"
                    type="button"
                    disabled
                    title="Skills require a verified Alpine capability registry."
                  >
                    <PuzzlePiece size={17} />
                    <span>
                      Add skill or tool<small>No verified registry yet</small>
                    </span>
                  </button>
                </div>
              )}
            </div>
            {running && (
              <button
                type="button"
                disabled={!draft.trim()}
                onClick={() => {
                  const prompt = draft.trim();
                  if (!prompt) return;
                  setDraft("");
                  onQueue(prompt, "follow-up");
                }}
              >
                Queue next
              </button>
            )}
          </div>
          <div className="composer-options">
            <span
              className="composer-runtime"
              title="Pi experimental agent adapter"
            >
              Pi
            </span>
            <select
              aria-label="Model for this task"
              value={activeModelRegistryId}
              disabled={
                running ||
                !installedModels.some(
                  (model) => model.state === "installed" && model.registryId,
                )
              }
              onChange={(event) => void onSelectModel(event.target.value)}
            >
              <option value="">Choose model</option>
              {installedModels
                .filter(
                  (model) => model.state === "installed" && model.registryId,
                )
                .map((model) => (
                  <option
                    key={`${model.localPath}:${model.filename}`}
                    value={model.registryId ?? ""}
                  >
                    {model.filename}
                  </option>
                ))}
            </select>
            <button
              className="send"
              type={running && !draft.trim() ? "button" : "submit"}
              aria-label={
                running && !draft.trim()
                  ? "Cancel task"
                  : running
                    ? "Steer task"
                    : "Run task"
              }
              disabled={
                taskRun?.state === "cancelling" ||
                (!running && (!draft.trim() || !project || !activeModel))
              }
              onClick={running && !draft.trim() ? onCancel : undefined}
            >
              {running && !draft.trim() ? (
                <StopCircle size={18} weight="fill" />
              ) : (
                <ArrowDown size={18} weight="bold" />
              )}
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}

function ApprovalCard({
  approval,
  onDecision,
}: {
  approval: ToolApproval;
  onDecision: (approval: ToolApproval, approved: boolean) => Promise<void>;
}) {
  const [deciding, setDeciding] = useState(false);
  const primary =
    approval.operation === "shell"
      ? String(approval.proposal.command ?? "")
      : String(approval.proposal.path ?? "");
  const decide = async (approved: boolean) => {
    setDeciding(true);
    try {
      await onDecision(approval, approved);
    } finally {
      setDeciding(false);
    }
  };
  return (
    <section className="approval-card">
      <div>
        <ShieldCheck size={18} />
        <span>
          <strong>
            {approval.operation === "shell"
              ? "Run command?"
              : "Apply exact edit?"}
          </strong>
          <small>Pi is paused until you decide</small>
        </span>
      </div>
      <code>{primary}</code>
      {approval.operation === "edit" && (
        <p>
          {String(approval.proposal.oldText ?? "").slice(0, 140)} →{" "}
          {String(approval.proposal.newText ?? "").slice(0, 140)}
        </p>
      )}
      <div>
        <button
          type="button"
          disabled={deciding}
          onClick={() => void decide(false)}
        >
          Deny
        </button>
        <button
          className="primary-button"
          type="button"
          disabled={deciding}
          onClick={() => void decide(true)}
        >
          {deciding ? "Deciding…" : "Approve once"}
        </button>
      </div>
    </section>
  );
}

function ModelsView({
  query,
  setQuery,
  search,
  state,
  error,
  models,
  selected,
  chooseModel,
  downloads,
  downloadsError,
  importState,
  onImport,
  defaultModel,
  onSelectInstalled,
}: {
  query: string;
  setQuery: (value: string) => void;
  search: (event: FormEvent) => void;
  state: "idle" | "loading" | "error";
  error: string | null;
  models: ModelSearchResult[];
  selected: ModelSearchResult | null;
  chooseModel: (model: ModelSearchResult) => void;
  downloads: DownloadedModel[];
  downloadsError: string | null;
  importState: {
    state: "idle" | "running" | "done" | "error";
    message?: string;
  };
  onImport: (sourcePath: string) => Promise<void>;
  defaultModel: BootstrapSnapshot["settings"]["defaultModel"];
  onSelectInstalled: (registryId: string) => Promise<void>;
}) {
  const [sourcePath, setSourcePath] = useState("");
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (sourcePath.trim()) void onImport(sourcePath.trim());
  };
  const installed = downloads.filter(
    (model) => model.state === "installed" && model.registryId,
  );
  const selectedRegistryId =
    defaultModel?.registryId ??
    installed.find(
      (model) =>
        model.filename === defaultModel?.filename &&
        model.repoId === defaultModel?.repoId &&
        (!defaultModel?.revision || model.revision === defaultModel.revision),
    )?.registryId ??
    "";
  return (
    <div className="content-view model-library">
      <div className="page-heading">
        <p className="eyebrow">Models</p>
        <h1>Model library</h1>
        <p>
          Choose what new tasks use, import a GGUF file, or find another model
          on Hugging Face—all in one place.
        </p>
      </div>
      <section className="library-section">
        <div className="section-heading">
          <div>
            <h2>On this machine</h2>
            <p>
              {installed.length
                ? `${installed.length} verified ${installed.length === 1 ? "model" : "models"} available`
                : "No verified models yet"}
            </p>
          </div>
          <label>
            Model for new tasks
            <select
              aria-label="Model for new tasks"
              value={selectedRegistryId}
              onChange={(event) => void onSelectInstalled(event.target.value)}
            >
              <option value="">Not selected</option>
              {installed.map((model) => (
                <option
                  key={`${model.localPath}:${model.filename}`}
                  value={model.registryId ?? ""}
                >
                  {model.filename}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="download-list">
          {downloads.map((model) => (
            <div key={`${model.localPath ?? "scan"}:${model.filename}`}>
              <div className="model-icon">
                <HardDrives size={18} />
              </div>
              <div>
                <strong>{model.filename}</strong>
                <span>
                  {formatBytes(model.sizeBytes)} ·{" "}
                  {model.source === "import" ? "Imported" : "Hugging Face"}
                  {model.revision
                    ? ` · revision ${model.revision.slice(0, 8)}`
                    : ""}
                </span>
              </div>
              <span className={`download-state ${model.state}`}>
                {model.filename === defaultModel?.filename
                  ? "default"
                  : friendlyDownloadState(model.state)}
              </span>
            </div>
          ))}
          {!downloads.length && !downloadsError && (
            <div className="empty-list">
              <HardDrives size={24} />
              <p>Import a GGUF file or search Hugging Face below.</p>
            </div>
          )}
        </div>
        {downloadsError && (
          <div className="error-banner">
            <WarningCircle size={16} />
            {downloadsError}
          </div>
        )}
        <form className="import-model" onSubmit={submit}>
          <input
            aria-label="Import GGUF path"
            value={sourcePath}
            onChange={(event) => setSourcePath(event.target.value)}
            placeholder="Absolute path to an existing .gguf"
          />
          <button
            type="submit"
            disabled={!sourcePath.trim() || importState.state === "running"}
          >
            {importState.state === "running" ? "Importing…" : "Import GGUF"}
          </button>
        </form>
        {importState.message && (
          <p
            className={
              importState.state === "error"
                ? "settings-message error"
                : "settings-message"
            }
          >
            {importState.message}
          </p>
        )}
      </section>
      <section className="library-section discover-models">
        <div className="section-heading">
          <div>
            <h2>Find on Hugging Face</h2>
            <p>
              Results are estimates until Alpine measures them on this machine.
            </p>
          </div>
        </div>
        <form
          className="model-search"
          onSubmit={search}
          aria-busy={state === "loading"}
        >
          <MagnifyingGlass size={18} />
          <input
            type="search"
            aria-label="Search Hugging Face"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search models or publishers"
          />
          <button type="submit">
            {state === "loading" && <CircleNotch className="spin" size={15} />}
            Search
          </button>
        </form>
        {error && (
          <div className="error-banner">
            <WarningCircle size={16} />
            {error}
          </div>
        )}
        <div className="model-results">
          {models.map((model) => (
            <button
              type="button"
              key={model.id}
              className={selected?.id === model.id ? "selected" : ""}
              onClick={() => void chooseModel(model)}
            >
              <div className="model-icon">
                <Sparkle size={17} />
              </div>
              <div>
                <strong>{model.id}</strong>
                <span>
                  {model.publisher} · {model.artifacts.length} GGUF{" "}
                  {model.artifacts.length === 1 ? "file" : "files"} · updated{" "}
                  {formatDate(model.lastModified)}
                </span>
              </div>
              <div className="model-stats">
                <span>{formatCount(model.downloads)} downloads</span>
                <small>
                  {model.revision
                    ? `revision ${model.revision.slice(0, 8)}`
                    : "revision unresolved"}
                </small>
              </div>
            </button>
          ))}
        </div>
        {state === "idle" && !models.length && (
          <div className="empty-list">
            <MagnifyingGlass size={22} />
            <p>Search for a GGUF model to compare size and fit.</p>
          </div>
        )}
      </section>
    </div>
  );
}

function AnalysisView({
  snapshot,
  state,
  evaluation,
  onRun,
  onEvaluate,
}: {
  snapshot: BootstrapSnapshot | null;
  state: {
    state: "idle" | "running" | "done" | "error";
    report?: RuntimeProbeReport;
    error?: string;
  };
  evaluation: {
    state: "idle" | "running" | "done" | "error";
    scope?: EvaluationScope;
    progress?: EvaluationProgress;
    report?: FullEvaluationSummary;
    error?: string;
  };
  onRun: () => Promise<void>;
  onEvaluate: (scope: EvaluationScope) => Promise<void>;
}) {
  const [scope, setScope] = useState<EvaluationScope>("candidate");
  const selectedMatchesRuntime = Boolean(
    snapshot?.settings.defaultModel &&
      snapshot.runtime.model &&
      snapshot.settings.defaultModel.filename.toLowerCase() ===
        snapshot.runtime.model.toLowerCase(),
  );
  const enabled =
    snapshot?.runtime.state !== "unconfigured" &&
    snapshot?.runtime.state !== "unavailable" &&
    selectedMatchesRuntime;
  const metrics = evaluation.report
    ? evaluationWorkloads(evaluation.report)
    : [];
  const analysisRunning =
    state.state === "running" || evaluation.state === "running";
  return (
    <div className="content-view analysis-view">
      <div className="page-heading">
        <p className="eyebrow">Measured results</p>
        <h1>Analysis</h1>
        <p>
          Check the active model or compare a bounded set of performance
          settings. Analysis never changes the model used for new tasks.
        </p>
      </div>
      <div className="analysis-grid">
        <section className="analysis-card">
          <div>
            <Gauge size={22} />
            <div>
              <strong>Quick model check</strong>
              <p>
                Starts the local model, checks an exact answer, and records how
                long it took.
              </p>
            </div>
          </div>
          <dl>
            <div>
              <dt>Model</dt>
              <dd>{snapshot?.runtime.model ?? "Not configured"}</dd>
            </div>
            <div>
              <dt>Profile</dt>
              <dd>{snapshot?.runtime.profile ?? "Not configured"}</dd>
            </div>
            <div>
              <dt>Evidence</dt>
              <dd>Measured check — not a recommendation</dd>
            </div>
          </dl>
          {!selectedMatchesRuntime && (
            <p className="analysis-warning">
              <WarningCircle size={15} />
              Choose the same model in Settings that the local runtime is using.
            </p>
          )}
          <button
            className="primary-button"
            type="button"
            disabled={!enabled || analysisRunning}
            onClick={() => void onRun()}
          >
            {state.state === "running" ? (
              <>
                <CircleNotch className="spin" size={16} />
                Running diagnostic…
              </>
            ) : (
              "Run measured diagnostic"
            )}
          </button>
        </section>
        <section className="analysis-card full-evaluation-card">
          <div>
            <Pulse size={22} />
            <div>
              <strong>Compare performance settings</strong>
              <p>
                Compares the current and candidate settings, recommends a change
                only when measurements improve, then restores the previous local
                model session.
              </p>
            </div>
          </div>
          <dl>
            <div>
              <dt>Scope</dt>
              <dd>
                <select
                  aria-label="Evaluation scope"
                  value={scope}
                  disabled={analysisRunning}
                  onChange={(event) =>
                    setScope(event.target.value as EvaluationScope)
                  }
                >
                  <option value="candidate">Standard · tune and verify</option>
                  <option value="validated">
                    Extended · add stability and coding checks
                  </option>
                  <option value="production">
                    Release · add rollback proof
                  </option>
                </select>
              </dd>
            </div>
            <div>
              <dt>Settings</dt>
              <dd>Stable vs faster candidate</dd>
            </div>
            <div>
              <dt>Current setup</dt>
              <dd>Never changed by analysis</dd>
            </div>
          </dl>
          <button
            className="primary-button"
            type="button"
            disabled={!enabled || analysisRunning}
            onClick={() => void onEvaluate(scope)}
          >
            {evaluation.state === "running" ? (
              <>
                <CircleNotch className="spin" size={16} />
                Running {evaluation.scope} evaluation…
              </>
            ) : (
              "Run full analysis"
            )}
          </button>
          {evaluation.progress && (
            <p className="evaluation-progress-note">
              {evaluation.progress.message}
            </p>
          )}
        </section>
      </div>
      {analysisRunning && (
        <div
          className="analysis-progress"
          role="progressbar"
          aria-label={
            state.state === "running"
              ? "Quick model check in progress"
              : "Full analysis in progress"
          }
          aria-valuetext={
            state.state === "running"
              ? "Starting the local model and measuring an exact response"
              : (evaluation.progress?.message ??
                `Running ${evaluation.scope ?? scope} evaluation`)
          }
        >
          <CircleNotch className="spin" size={17} />
          <div>
            <strong>
              {state.state === "running"
                ? "Checking the active model"
                : "Comparing performance settings"}
            </strong>
            <span>
              {state.state === "running"
                ? "Starting the local model and measuring an exact response."
                : (evaluation.progress?.message ??
                  "Preparing the bounded evaluation and restoration check.")}
            </span>
          </div>
        </div>
      )}
      {state.report && (
        <section className="probe-report">
          <p className="eyebrow">Diagnostic result</p>
          <div>
            <strong>
              {state.report.qualityPass
                ? "Exact output passed"
                : "Exact output failed"}
            </strong>
            <span>
              {state.report.latencyMs} ms end-to-end ·{" "}
              {state.report.outputTokens ?? "unknown"} output tokens
            </span>
          </div>
          <p>{state.report.evidenceLabel}</p>
        </section>
      )}
      {state.error && (
        <div className="error-banner">
          <WarningCircle size={16} />
          {state.error}
        </div>
      )}
      {evaluation.report && (
        <section className="evaluation-report">
          <div className="evaluation-decision">
            <div>
              <p className="eyebrow">{evaluation.report.scope} evaluation</p>
              <h2>{evaluation.report.decision}</h2>
              <p>{evaluation.report.recommendation}</p>
            </div>
            <span className={`decision-pill ${evaluation.report.decision}`}>
              {evaluation.report.selectedProfile ?? "retain baseline"}
            </span>
          </div>
          <div className="metric-grid">
            {metrics.map((metric) => (
              <article key={metric.name}>
                <strong>{metric.label}</strong>
                <span>{metric.value}</span>
                <small>{metric.detail}</small>
              </article>
            ))}
          </div>
          <div className="gate-grid">
            <EvidenceGate
              title="Correctness"
              value={
                summaryFlag(evaluation.report, "all_quality_pass")
                  ? "Passed"
                  : "Not proven"
              }
            />
            <EvidenceGate
              title="Determinism"
              value={
                summaryFlag(evaluation.report, "all_deterministic")
                  ? "Passed"
                  : "Not proven"
              }
            />
            <EvidenceGate
              title="Peak VRAM"
              value={evaluationResourceMetric(
                evaluation.report,
                "vram_peak_mib",
              )}
            />
            <EvidenceGate
              title="Shared-memory spill"
              value={evaluationResourceMetric(
                evaluation.report,
                "shared_memory_peak_mib",
              )}
            />
            <EvidenceGate
              title="Same-process stability"
              value={
                evaluation.report.sameProcessRequests
                  ? `${evaluation.report.sameProcessRequests} requests`
                  : "Not run"
              }
            />
            <EvidenceGate
              title="Clean restarts"
              value={
                evaluation.report.cleanRestarts
                  ? `${evaluation.report.cleanRestarts} / ${evaluation.report.cleanRestarts}`
                  : "Not run"
              }
            />
            <EvidenceGate
              title="Near-limit context"
              value={
                evaluation.report.nearLimitContextTokens
                  ? `${evaluation.report.nearLimitContextTokens} tokens`
                  : "Not run"
              }
            />
            <EvidenceGate
              title="Tool task"
              value={
                evaluation.report.goldenToolCalls != null
                  ? `${evaluation.report.goldenToolCalls} calls · ${evaluation.report.goldenToolFailures} recovered failures`
                  : "Not run"
              }
            />
            <EvidenceGate
              title="Rollback"
              value={
                evaluation.report.rollbackProved
                  ? `${evaluation.report.rollbackProfile} proved`
                  : `${evaluation.report.rollbackProfile} preserved`
              }
            />
            <EvidenceGate
              title="Prior Session"
              value={
                evaluation.report.priorSessionRestored
                  ? "Restored"
                  : "Not proven"
              }
            />
          </div>
          <footer>
            <span>
              Plan {evaluation.report.planId} · sha256{" "}
              {evaluation.report.planSha256.slice(0, 12)}…
            </span>
            <code>{evaluation.report.artifactPath}</code>
          </footer>
        </section>
      )}
      {evaluation.error && (
        <div className="error-banner">
          <WarningCircle size={16} />
          {evaluation.error}
        </div>
      )}
    </div>
  );
}

function EvidenceGate({ title, value }: { title: string; value: string }) {
  return (
    <div>
      <span>{title}</span>
      <strong>{value}</strong>
    </div>
  );
}

function SettingsView({
  snapshot,
  downloads,
  runtimeControl,
  onRuntimeControl,
  onSave,
  onUseActive,
  onSelectInstalled,
  onClearBrowserData,
}: {
  snapshot: BootstrapSnapshot | null;
  downloads: DownloadedModel[];
  runtimeControl: { state: "idle" | "running" | "error"; message?: string };
  onRuntimeControl: (action: "start" | "stop") => Promise<void>;
  onSave: (update: SettingsUpdate) => Promise<void>;
  onUseActive: () => Promise<void>;
  onSelectInstalled: (registryId: string) => Promise<void>;
  onClearBrowserData: () => Promise<void>;
}) {
  const [installRoot, setInstallRoot] = useState("");
  const [evaluationRoot, setEvaluationRoot] = useState("");
  const [profile, setProfile] = useState("stable-16k");
  const [localMetrics, setLocalMetrics] = useState(true);
  const [section, setSection] = useState<
    "general" | "runtime" | "browser" | "safety" | "privacy"
  >("general");
  const [saveState, setSaveState] = useState<{
    state: "idle" | "saving" | "done" | "error";
    message?: string;
  }>({ state: "idle" });
  const [browserDataState, setBrowserDataState] = useState<{
    clearing: boolean;
    message?: string;
    error?: boolean;
  }>({ clearing: false });
  useEffect(() => {
    if (snapshot) {
      setInstallRoot(snapshot.settings.installRoot);
      setEvaluationRoot(snapshot.settings.evaluationRepositoryRoot);
      setProfile(snapshot.settings.defaultProfile);
      setLocalMetrics(snapshot.settings.localMetricsEnabled);
    }
  }, [snapshot]);
  const save = async (event: FormEvent) => {
    event.preventDefault();
    setSaveState({ state: "saving" });
    try {
      await onSave({
        installRoot,
        evaluationRepositoryRoot: evaluationRoot,
        defaultProfile: profile,
        localMetricsEnabled: localMetrics,
        browserAllowedHosts: snapshot?.settings.browserAllowedHosts ?? [],
      });
      setSaveState({
        state: "done",
        message: "Settings saved and runtime rechecked.",
      });
    } catch (error) {
      setSaveState({ state: "error", message: errorMessage(error) });
    }
  };
  const clearBrowserData = async () => {
    setBrowserDataState({ clearing: true });
    try {
      await onClearBrowserData();
      setBrowserDataState({
        clearing: false,
        message: "Browsing data cleared.",
      });
    } catch (error) {
      setBrowserDataState({
        clearing: false,
        message: errorMessage(error),
        error: true,
      });
    }
  };
  const profiles = Array.from(
    new Set([profile, ...(snapshot?.runtime.availableProfiles ?? [])]),
  );
  const installed = downloads.filter(
    (model) => model.state === "installed" && model.registryId,
  );
  const selectedRegistryId =
    snapshot?.settings.defaultModel?.registryId ??
    installed.find(
      (model) =>
        model.filename === snapshot?.settings.defaultModel?.filename &&
        model.repoId === snapshot?.settings.defaultModel?.repoId &&
        (!snapshot?.settings.defaultModel?.revision ||
          model.revision === snapshot.settings.defaultModel.revision),
    )?.registryId ??
    "";
  return (
    <form className="content-view settings-view" onSubmit={save}>
      <div className="page-heading">
        <p className="eyebrow">Preferences</p>
        <h1>Settings</h1>
        <p>
          Keep everyday choices simple. Model runtime, safety, and privacy
          controls stay in their own sections.
        </p>
      </div>
      <div className="settings-layout">
        <nav className="settings-nav" aria-label="Settings sections">
          {(
            ["general", "runtime", "browser", "safety", "privacy"] as const
          ).map((value) => (
            <button
              key={value}
              type="button"
              className={section === value ? "active" : ""}
              aria-current={section === value ? "page" : undefined}
              onClick={() => setSection(value)}
            >
              <span>
                {value === "general"
                  ? "General"
                  : value === "runtime"
                    ? "Runtime"
                    : value === "browser"
                      ? "Browser"
                      : value === "safety"
                        ? "Safety"
                        : "Privacy"}
              </span>
              <CaretRight size={15} />
            </button>
          ))}
        </nav>
        <div className="settings-panel">
          {section === "general" && (
            <section className="settings-group">
              <h2>General</h2>
              <SettingRow
                title="Agent runtime"
                copy="Pi is the first agent integration. Alpine keeps project access, approvals, and task history."
              >
                <span className="runtime-choice">
                  <strong>Pi</strong>
                  <small>Experimental adapter</small>
                </span>
              </SettingRow>
              <PiCapabilitySummary />
              <SettingRow
                title="Model for new tasks"
                copy="Choose from models already verified on this machine."
              >
                <div className="setting-action">
                  <select
                    aria-label="Default model"
                    value={selectedRegistryId}
                    onChange={(event) =>
                      void onSelectInstalled(event.target.value)
                    }
                  >
                    <option value="">Not selected</option>
                    {installed.map((model) => (
                      <option
                        key={`${model.localPath}:${model.filename}`}
                        value={model.registryId ?? ""}
                      >
                        {model.filename}
                      </option>
                    ))}
                  </select>
                  {snapshot?.runtime.model &&
                    snapshot.runtime.model !==
                      snapshot.settings.defaultModel?.filename && (
                      <button type="button" onClick={() => void onUseActive()}>
                        Use active model
                      </button>
                    )}
                </div>
              </SettingRow>
              <SettingRow
                title="Performance preset"
                copy="Used the next time the local model starts."
              >
                <select
                  aria-label="Default profile"
                  value={profile}
                  onChange={(event) => setProfile(event.target.value)}
                >
                  {profiles.map((value) => (
                    <option key={value} value={value}>
                      {value}
                    </option>
                  ))}
                </select>
              </SettingRow>
            </section>
          )}
          {section === "runtime" && (
            <section className="settings-group">
              <h2>Runtime</h2>
              <SettingRow
                title="Local model"
                copy={
                  runtimeControl.message ??
                  friendlyRuntimeDetail(snapshot?.runtime.detail)
                }
              >
                <div className="runtime-controls">
                  <span
                    className={`settings-status ${runtimeControl.state === "error" ? "unavailable" : (snapshot?.runtime.state ?? "loading")}`}
                  >
                    {runtimeControl.state === "running"
                      ? "working"
                      : friendlyRuntimeState(snapshot?.runtime.state)}
                  </span>
                  <button
                    type="button"
                    disabled={
                      !snapshot ||
                      runtimeControl.state === "running" ||
                      snapshot.runtime.state === "running" ||
                      snapshot.runtime.state === "unconfigured" ||
                      snapshot.runtime.state === "unavailable"
                    }
                    onClick={() => void onRuntimeControl("start")}
                  >
                    Start
                  </button>
                  <button
                    type="button"
                    disabled={
                      !snapshot ||
                      runtimeControl.state === "running" ||
                      snapshot.runtime.state !== "running"
                    }
                    onClick={() => void onRuntimeControl("stop")}
                  >
                    Stop
                  </button>
                </div>
              </SettingRow>
              <SettingRow
                title="Model storage"
                copy="The folder Alpine manages for local models and runtime files."
              >
                <input
                  aria-label="Alpine installation root"
                  value={installRoot}
                  onChange={(event) => setInstallRoot(event.target.value)}
                />
              </SettingRow>
              <SettingRow
                title="Evaluation project"
                copy="The Alpine repository containing versioned evaluation plans."
              >
                <input
                  aria-label="Evaluation repository root"
                  value={evaluationRoot}
                  onChange={(event) => setEvaluationRoot(event.target.value)}
                />
              </SettingRow>
            </section>
          )}
          {section === "browser" && (
            <section className="settings-group">
              <h2>Browser</h2>
              <SettingRow
                title="Website access"
                copy="Local pages open directly. Alpine asks before a tab visits each new external site."
              >
                <span className="settings-status configured">
                  Ask before new websites
                </span>
              </SettingRow>
              <SettingRow
                title="Browser profile"
                copy="Sign-ins, cookies, and downloads use an Alpine-owned browser profile."
              >
                <span className="settings-status configured">
                  Separate from your regular browser
                </span>
              </SettingRow>
              <SettingRow
                title="Browsing data"
                copy="Remove cookies, cache, local storage, and saved sessions from Alpine's browser profile."
              >
                <div className="setting-action">
                  <button
                    type="button"
                    disabled={browserDataState.clearing}
                    onClick={() => void clearBrowserData()}
                  >
                    {browserDataState.clearing
                      ? "Clearing…"
                      : "Clear browsing data"}
                  </button>
                  {browserDataState.message && (
                    <span
                      className={
                        browserDataState.error
                          ? "settings-message error"
                          : "settings-message"
                      }
                      role="status"
                    >
                      {browserDataState.message}
                    </span>
                  )}
                </div>
              </SettingRow>
            </section>
          )}
          {section === "safety" && (
            <section className="settings-group">
              <h2>Safety</h2>
              <SettingRow
                title="File changes"
                copy="Alpine asks before each exact edit."
              >
                <span className="settings-status configured">
                  Ask every time
                </span>
              </SettingRow>
              <SettingRow
                title="Commands"
                copy="Alpine asks before each command and records the result."
              >
                <span className="settings-status configured">
                  Ask every time
                </span>
              </SettingRow>
              <SettingRow
                title="Execution identity"
                copy="Approved commands run inside the selected project as your current Windows user. Alpine is not a sandbox."
              >
                <span className="settings-status identity-warning">
                  Windows user
                </span>
              </SettingRow>
              <SettingRow
                title="Project access"
                copy="Reading and searching stay inside the project selected in the left rail."
              >
                <span className="settings-status configured">Project only</span>
              </SettingRow>
            </section>
          )}
          {section === "privacy" && (
            <section className="settings-group">
              <h2>Privacy</h2>
              <SettingRow
                title="Performance measurements"
                copy="Store local timings and counts. Prompts, credentials, and project contents are excluded."
              >
                <button
                  className={`toggle ${localMetrics ? "on" : ""}`}
                  type="button"
                  role="switch"
                  aria-label="Local performance measurements"
                  aria-checked={localMetrics}
                  onClick={() => setLocalMetrics((value) => !value)}
                >
                  <span />
                </button>
              </SettingRow>
              <SettingRow
                title="Diagnostics"
                copy="Task content stays on this machine and is not included in performance measurements."
              >
                <span className="settings-status configured">Local only</span>
              </SettingRow>
            </section>
          )}
          {saveState.message && (
            <p
              className={
                saveState.state === "error"
                  ? "settings-message error"
                  : "settings-message"
              }
            >
              {saveState.message}
            </p>
          )}
          <button
            className="primary-button settings-save"
            type="submit"
            disabled={!snapshot || saveState.state === "saving"}
          >
            {saveState.state === "saving" ? "Saving…" : "Save changes"}
          </button>
        </div>
      </div>
    </form>
  );
}

function SettingRow({
  title,
  copy,
  children,
}: {
  title: string;
  copy: string;
  children: React.ReactNode;
}) {
  return (
    <div className="setting-row">
      <div>
        <strong>{title}</strong>
        <p>{copy}</p>
      </div>
      <div>{children}</div>
    </div>
  );
}

function PiCapabilitySummary() {
  const available = PI_RUNTIME_CAPABILITIES.filter(
    (capability) => capability.status === "available",
  ).length;
  return (
    <details className="capability-disclosure">
      <summary>
        <span>Pi feature coverage</span>
        <strong>{available} available · gaps shown explicitly</strong>
      </summary>
      <div className="capability-list">
        {PI_RUNTIME_CAPABILITIES.map((capability) => (
          <article key={capability.id}>
            <div>
              <strong>{capability.label}</strong>
              <span className={`capability-status ${capability.status}`}>
                {capability.status}
              </span>
            </div>
            <p>{capability.detail}</p>
          </article>
        ))}
      </div>
    </details>
  );
}

function ContextInspector({
  browser,
  tab,
  setTab,
  snapshot,
  bootstrapError,
  selected,
  selectedArtifact,
  selectedInstalled,
  assessment,
  placement,
  metrics,
  defaultSaved,
  saveDefault,
  chooseArtifact,
  downloadState,
  downloadProgress,
  downloadSelected,
  cancelDownload,
  taskDetail,
  files,
  workspaceRead,
  clearWorkspaceRead,
  openFile,
  hardwareLine,
}: {
  browser: DesktopClient["browser"];
  tab: InspectorTab;
  setTab: (tab: InspectorTab) => void;
  snapshot: BootstrapSnapshot | null;
  bootstrapError: string | null;
  selected: ModelSearchResult | null;
  selectedArtifact: ModelSearchResult["artifacts"][number] | undefined;
  selectedInstalled: boolean;
  assessment: ModelAssessment | null;
  placement: PlacementPlan | null;
  metrics: RendererMetric[];
  defaultSaved: boolean;
  saveDefault: () => Promise<void>;
  chooseArtifact: (filename: string) => Promise<void>;
  downloadState: {
    state: "idle" | "running" | "done" | "error";
    message?: string;
  };
  downloadProgress: DownloadProgress | null;
  downloadSelected: () => Promise<void>;
  cancelDownload: () => Promise<void>;
  taskDetail: TaskDetail | null;
  files: WorkspaceEntry[];
  workspaceRead: WorkspaceRead | null;
  clearWorkspaceRead: () => void;
  openFile: (path: string) => Promise<void>;
  hardwareLine: string;
}) {
  const lastDiff = latestToolDetails(taskDetail?.events ?? [], "edit_file") as {
    diff?: string;
    path?: string;
  } | null;
  const lastShell = latestToolDetails(
    taskDetail?.events ?? [],
    "run_command",
  ) as {
    command?: string;
    stdout?: string;
    stderr?: string;
    exitCode?: number;
    durationMs?: number;
  } | null;
  const tabs: Array<{
    value: InspectorTab;
    label: string;
    icon: React.ReactNode;
  }> = [
    { value: "system", label: "System", icon: <Cpu size={14} /> },
    { value: "files", label: "Files", icon: <FileText size={14} /> },
    { value: "changes", label: "Changes", icon: <GitDiff size={14} /> },
    {
      value: "terminal",
      label: "Terminal",
      icon: <TerminalWindow size={14} />,
    },
    { value: "browser", label: "Browser", icon: <Browser size={14} /> },
  ];
  return (
    <aside
      id="context-inspector"
      className={`inspector ${tab === "browser" ? "browser-active" : ""}`}
      aria-label="Context inspector"
    >
      <div className="inspector-tabs">
        {tabs.map((item) => (
          <button
            key={item.value}
            type="button"
            className={tab === item.value ? "active" : ""}
            aria-label={item.label}
            title={item.label}
            onClick={() => setTab(item.value)}
          >
            {item.icon}
            <span>{item.label}</span>
          </button>
        ))}
      </div>
      <div className="inspector-header">
        <span>{tabs.find((item) => item.value === tab)?.label}</span>
        <small>
          {tab === "browser"
            ? "Own profile · asks before new sites"
            : (taskDetail?.task.modelFilename ??
              snapshot?.settings.defaultModel?.filename ??
              "No model selected")}
        </small>
      </div>
      <div className="inspector-content">
        {tab === "system" && (
          <>
            <section className="hardware-card">
              <div className="card-label">
                <Cpu size={15} />
                This machine
              </div>
              {snapshot ? (
                <>
                  <strong>
                    {snapshot.hardware.gpu ?? snapshot.hardware.cpu}
                  </strong>
                  <dl>
                    <div>
                      <dt>System</dt>
                      <dd>
                        {snapshot.hardware.platform}
                        {snapshot.hardware.osVersion
                          ? ` ${snapshot.hardware.osVersion}`
                          : ""}{" "}
                        · {snapshot.hardware.architecture}
                      </dd>
                    </div>
                    <div>
                      <dt>Processor</dt>
                      <dd>
                        {snapshot.hardware.physicalCores
                          ? `${snapshot.hardware.physicalCores} cores · `
                          : ""}
                        {snapshot.hardware.logicalProcessors} logical processors
                      </dd>
                    </div>
                    <div>
                      <dt>Graphics memory</dt>
                      <dd>{formatBytes(snapshot.hardware.vramBytes)}</dd>
                    </div>
                    <div>
                      <dt>System memory</dt>
                      <dd>{formatBytes(snapshot.hardware.memoryBytes)}</dd>
                    </div>
                    <div>
                      <dt>Model compute</dt>
                      <dd>
                        {snapshot.hardware.computeDevices.length
                          ? `${snapshot.hardware.computeDevices.length} CUDA ${snapshot.hardware.computeDevices.length === 1 ? "device" : "devices"}`
                          : "CPU only"}
                      </dd>
                    </div>
                    <div>
                      <dt>Driver</dt>
                      <dd>{snapshot.hardware.driver ?? "Not reported"}</dd>
                    </div>
                  </dl>
                  <p className="hardware-policy">
                    Windows chooses interface graphics. Alpine assigns
                    local-model work only to measured compute devices.
                  </p>
                </>
              ) : (
                <p className="muted">
                  {bootstrapError ?? "Checking local hardware…"}
                </p>
              )}
            </section>
            {metrics.length > 0 && (
              <details className="metrics-details">
                <summary>Performance details</summary>
                <section className="metrics-card">
                  {metrics.map((metric) => (
                    <div key={metric.label}>
                      <span>{metric.label}</span>
                      <strong>{metric.value}</strong>
                      <small>{metric.detail}</small>
                    </div>
                  ))}
                </section>
              </details>
            )}
            {selected ? (
              <section className="selection-card">
                <p className="eyebrow">Model details</p>
                <strong>{selected.id}</strong>
                {selected.artifacts.length > 1 ? (
                  <select
                    className="artifact-select"
                    aria-label="Model artifact"
                    value={selectedArtifact?.filename ?? ""}
                    onChange={(event) =>
                      void chooseArtifact(event.target.value)
                    }
                  >
                    {selected.artifacts.map((artifact) => (
                      <option key={artifact.filename} value={artifact.filename}>
                        {artifact.filename} · {formatBytes(artifact.sizeBytes)}
                      </option>
                    ))}
                  </select>
                ) : (
                  <span>{selectedArtifact?.filename ?? "No GGUF file"}</span>
                )}
                {assessment ? (
                  <div
                    className={`fit-badge ${assessment.status === "unlikely-to-fit" ? "warning" : ""}`}
                  >
                    {assessment.status === "unlikely-to-fit" ? (
                      <WarningCircle size={14} />
                    ) : (
                      <Check size={14} />
                    )}
                    {fitLabel(assessment)}
                  </div>
                ) : (
                  <div className="fit-badge neutral">
                    Size information unavailable
                  </div>
                )}
                {placement && (
                  <div className="placement-plan">
                    <p className="eyebrow">Suggested first run</p>
                    {placement.candidates.map((candidate) => (
                      <div
                        className={
                          candidate.id === placement.recommendedId
                            ? "recommended"
                            : ""
                        }
                        key={candidate.id}
                      >
                        <span>{friendlyPlacementLabel(candidate.id)}</span>
                        <strong>
                          {candidate.viable
                            ? `${candidate.gpuResidencyPercent}% graphics`
                            : "Not a safe fit"}
                        </strong>
                      </div>
                    ))}
                    <small>
                      Estimate only. Analysis measures the real result.
                    </small>
                  </div>
                )}
                {assessment && (
                  <p className="evidence">
                    Estimate, not a performance result.
                  </p>
                )}
                <button
                  className="primary-button"
                  type="button"
                  onClick={() => void saveDefault()}
                  disabled={!selectedArtifact || !selectedInstalled}
                >
                  {defaultSaved ? (
                    <>
                      <Check size={16} />
                      Used for new tasks
                    </>
                  ) : selectedInstalled ? (
                    "Use for new tasks"
                  ) : (
                    "Download first"
                  )}
                </button>
                <button
                  className="secondary-button"
                  type="button"
                  onClick={() =>
                    void (downloadState.state === "running"
                      ? cancelDownload()
                      : downloadSelected())
                  }
                >
                  {downloadState.state === "running" ? (
                    <>
                      <CircleNotch className="spin" size={16} />
                      Cancel download
                    </>
                  ) : (
                    <>
                      <DownloadSimple size={16} />
                      Download
                    </>
                  )}
                </button>
                {downloadProgress && downloadState.state === "running" && (
                  <div className="download-progress">
                    <span
                      style={{
                        width: downloadProgress.totalBytes
                          ? `${Math.min(100, (downloadProgress.bytesWritten / downloadProgress.totalBytes) * 100)}%`
                          : "18%",
                      }}
                    />
                    <small>
                      {downloadProgress.state} ·{" "}
                      {formatBytes(downloadProgress.bytesWritten)} /{" "}
                      {formatBytes(downloadProgress.totalBytes)}
                    </small>
                  </div>
                )}
                {downloadState.message && (
                  <p
                    className={
                      downloadState.state === "error"
                        ? "download-note error"
                        : "download-note"
                    }
                  >
                    {downloadState.message}
                  </p>
                )}
              </section>
            ) : (
              <section className="inspector-empty">
                <Code size={20} />
                <p>Choose a model or open a task to see details here.</p>
              </section>
            )}
          </>
        )}
        {tab === "files" && (
          <section className="file-inspector">
            {workspaceRead ? (
              <>
                <button
                  className="back-link"
                  type="button"
                  onClick={clearWorkspaceRead}
                >
                  <ArrowDown size={14} />
                  Back to files
                </button>
                <p className="file-path">{workspaceRead.path}</p>
                <pre>{workspaceRead.content}</pre>
                {workspaceRead.truncated && (
                  <small>
                    Showing {workspaceRead.startLine}–{workspaceRead.endLine} of{" "}
                    {workspaceRead.totalLines}
                  </small>
                )}
              </>
            ) : files.length ? (
              files.map((entry) => (
                <button
                  type="button"
                  key={entry.path}
                  disabled={entry.kind !== "file"}
                  onClick={() => void openFile(entry.path)}
                >
                  {entry.kind === "directory" ? (
                    <Folder size={14} />
                  ) : (
                    <FileText size={14} />
                  )}
                  <span>{entry.path}</span>
                  {entry.kind === "file" && (
                    <small>{formatCompactBytes(entry.sizeBytes)}</small>
                  )}
                </button>
              ))
            ) : (
              <div className="inspector-empty">
                <Folder size={20} />
                <p>Open a task to browse files in its project.</p>
              </div>
            )}
          </section>
        )}
        {tab === "changes" && (
          <section className="artifact-panel">
            {lastDiff?.diff ? (
              <>
                <p className="eyebrow">Latest edit</p>
                <pre className="diff-output">{lastDiff.diff}</pre>
              </>
            ) : (
              <div className="inspector-empty">
                <GitDiff size={20} />
                <p>Approved edits appear here.</p>
              </div>
            )}
          </section>
        )}
        {tab === "terminal" && (
          <section className="artifact-panel">
            {lastShell ? (
              <>
                <p className="eyebrow">Latest command</p>
                <code>{lastShell.command}</code>
                <pre className="terminal-output">
                  {lastShell.stdout}
                  {lastShell.stderr ? `\n[stderr]\n${lastShell.stderr}` : ""}
                </pre>
                <small>
                  Exit {lastShell.exitCode} · {lastShell.durationMs} ms
                </small>
              </>
            ) : (
              <div className="inspector-empty">
                <TerminalWindow size={20} />
                <p>Command output appears here.</p>
              </div>
            )}
          </section>
        )}
        {tab === "browser" && <BrowserSurface browser={browser} />}
      </div>
      <div className="inspector-footer">
        <section className="runtime-card">
          <span
            className={`status-dot ${snapshot?.runtime.state === "unconfigured" || snapshot?.runtime.state === "unavailable" ? "warning" : ""}`}
          />
          <div>
            <strong>
              {snapshot
                ? `Local model ${friendlyRuntimeState(snapshot.runtime.state)}`
                : "Checking local model"}
            </strong>
            <small>{snapshot?.runtime.profile ?? "No profile"}</small>
          </div>
        </section>
        <p className="hardware-line">{hardwareLine}</p>
      </div>
    </aside>
  );
}

function latestToolDetails(events: TaskEvent[], toolName: string) {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (event.kind !== "tool.finished") continue;
    const payload = event.payload as { toolName?: string; details?: unknown };
    if (payload.toolName === toolName) return payload.details ?? null;
  }
  return null;
}

function taskTitle(prompt: string) {
  const firstLine = prompt.split(/\r?\n/, 1)[0]?.trim() || "New local task";
  return firstLine.length > 72 ? `${firstLine.slice(0, 69)}…` : firstLine;
}

function evaluationSummary(
  report: FullEvaluationSummary,
): Record<string, unknown> {
  const evidence = report.finalEvidence ?? {};
  return (
    (evidence.result_summary as Record<string, unknown> | undefined) ??
    (evidence.resultSummary as Record<string, unknown> | undefined) ??
    {}
  );
}

function summaryFlag(report: FullEvaluationSummary, key: string) {
  return evaluationSummary(report)[key] === true;
}

function evaluationResourceMetric(report: FullEvaluationSummary, key: string) {
  const resources = evaluationSummary(report).resources as
    | Record<string, unknown>
    | undefined;
  const value = resources?.[key];
  return typeof value === "number" ? `${value.toFixed(0)} MiB` : "Not captured";
}

function evaluationWorkloads(report: FullEvaluationSummary) {
  const workloads =
    (evaluationSummary(report).workloads as
      | Record<string, Record<string, unknown>>
      | undefined) ?? {};
  const definitions = [
    { name: "prefill-4k", label: "Prompt processing", metric: "prefill_tps" },
    { name: "novel-256", label: "Novel decode", metric: "decode_tps" },
    {
      name: "repeat-code-256",
      label: "Repeated-token decode",
      metric: "decode_tps",
    },
    {
      name: "structured-json-128",
      label: "Structured decode",
      metric: "decode_tps",
    },
  ];
  return definitions.map((definition) => {
    const workload = workloads[definition.name] ?? {};
    const metric =
      (workload[definition.metric] as Record<string, unknown> | undefined) ??
      {};
    const median = typeof metric.median === "number" ? metric.median : null;
    const quality =
      typeof workload.quality_pass_rate === "number"
        ? workload.quality_pass_rate
        : null;
    const deterministic = workload.deterministic === true;
    return {
      name: definition.name,
      label: definition.label,
      value: median == null ? "Not measured" : `${median.toFixed(1)} tok/s`,
      detail: `${quality == null ? "quality unknown" : `${Math.round(quality * 100)}% quality`} · ${deterministic ? "deterministic" : "variance observed"}`,
    };
  });
}

function formatDate(value: string | null) {
  if (!value) return "unknown";
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "unknown" : date.toLocaleDateString();
}

function formatCompactBytes(value: number) {
  if (value < 1024) return `${value} B`;
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(0)} KB`;
  return `${(value / 1024 ** 2).toFixed(1)} MB`;
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function taskFailureMessage(error: unknown) {
  const message = errorMessage(error).trim();
  if (/local runtime is unavailable/i.test(message))
    return "The local model session is unavailable. Start it in Settings, then try again.";
  if (/connection error|failed to fetch|could not connect|connection refused/i.test(message))
    return "Alpine couldn't reach the local model. Check the verified local session in Settings, then try again.";
  const detail = message.replace(/[.!?]+$/, "");
  return `${detail || "The task stopped unexpectedly"}. Check the local model session, then try again.`;
}
function titleFor(view: View) {
  return (
    {
      task: "New task",
      models: "Models",
      analysis: "Analysis",
      settings: "Settings",
    } as const
  )[view];
}

function friendlyTaskStatus(status: DesktopTask["status"]) {
  return (
    {
      draft: "Draft",
      running: "Working",
      cancelling: "Stopping",
      completed: "Done",
      cancelled: "Stopped",
      failed: "Needs attention",
      interrupted: "Interrupted",
    } as const
  )[status];
}

function friendlyDownloadState(state: DownloadedModel["state"]) {
  return state === "installed" ? "Ready" : "Incomplete";
}

function friendlyRuntimeState(
  state: BootstrapSnapshot["runtime"]["state"] | undefined,
) {
  return (
    {
      running: "running",
      configured: "ready",
      unavailable: "unavailable",
      unconfigured: "not set up",
    } as const
  )[state ?? "unconfigured"];
}

function friendlyRuntimeDetail(detail: string | undefined) {
  if (!detail) return "Checking the local model runtime.";
  return detail
    .replace(/control plane/gi, "local model runtime")
    .replace(/Inference Session/g, "local model session")
    .replace(/verified /gi, "");
}

function friendlyPlacementLabel(id: PlacementPlan["candidates"][number]["id"]) {
  return (
    {
      "full-gpu": "Graphics card",
      "balanced-hybrid": "Graphics + CPU",
      "conservative-hybrid": "Low-memory mix",
      "cpu-only": "CPU only",
    } as const
  )[id];
}
