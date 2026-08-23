import {
  ArrowDown,
  Browser,
  CaretDown,
  Check,
  CircleNotch,
  CloudArrowDown,
  Code,
  Cpu,
  DownloadSimple,
  FileText,
  Folder,
  Gauge,
  GearSix,
  GitDiff,
  HardDrives,
  House,
  MagnifyingGlass,
  Plus,
  Pulse,
  ShieldCheck,
  Sparkle,
  StopCircle,
  TerminalWindow,
  WarningCircle,
  X,
} from "@phosphor-icons/react";
import type { AgentEvent, AgentMessage } from "@earendil-works/pi-agent-core";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import type { PiHarness as PiHarnessInstance } from "./harness/pi";
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

type View = "task" | "models" | "downloads" | "analysis" | "browser" | "settings";
type InspectorTab = "system" | "files" | "changes" | "terminal" | "browser";
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
  new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value);

function collectRendererMetrics(enabled = true): RendererMetric[] {
  if (!enabled) return [];
  const rows: RendererMetric[] = [];
  const bootstrap = performance.getEntriesByName("alpine:bootstrap", "measure").at(-1);
  if (bootstrap) rows.push({ label: "Ready", value: `${Math.round(bootstrap.duration)} ms`, detail: "renderer start through local bootstrap" });
  const piLaunch = performance.getEntriesByName("alpine:pi-launch", "measure").at(-1);
  if (piLaunch) rows.push({ label: "Pi launch", value: `${Math.round(piLaunch.duration)} ms`, detail: "SDK import and local launch resolution" });
  const firstEvent = performance.getEntriesByName("alpine:stream:first-event", "measure").at(-1);
  if (firstEvent) rows.push({ label: "First stream event", value: `${Math.round(firstEvent.duration)} ms`, detail: "Pi prompt through first text delta" });
  const stream = performance.getEntriesByName("alpine:stream:duration", "measure").at(-1);
  if (stream) rows.push({ label: "Stream duration", value: `${Math.round(stream.duration)} ms`, detail: "complete Pi prompt lifecycle" });
  const resources = performance.getEntriesByType("resource") as PerformanceResourceTiming[];
  const clientBytes = resources
    .filter((entry) => entry.initiatorType === "script" || entry.initiatorType === "link")
    .reduce((total, entry) => total + (entry.transferSize || entry.encodedBodySize || 0), 0);
  if (clientBytes) rows.push({ label: "Client assets", value: formatCompactBytes(clientBytes), detail: "current transferred scripts and styles" });
  const memory = (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory;
  if (memory?.usedJSHeapSize) rows.push({ label: "Renderer heap", value: formatCompactBytes(memory.usedJSHeapSize), detail: "current JavaScript heap, when exposed by the webview" });
  rows.push({ label: "Long tasks", value: String(performance.getEntriesByType("longtask").length), detail: "renderer tasks longer than 50 ms" });
  return rows;
}

const fitLabel = (assessment: ModelAssessment) => {
  switch (assessment.status) {
    case "fits-gpu-with-headroom":
      return `Fits GPU with ${formatBytes(assessment.headroomBytes)} headroom`;
    case "fits-gpu-tight":
      return "Fits GPU, with limited headroom";
    case "fits-with-cpu-offload":
      return "Fits system memory with CPU offload";
    case "unlikely-to-fit":
      return "Unlikely to fit this machine";
  }
};

export function App({ desktop }: { desktop: DesktopClient }) {
  const [view, setView] = useState<View>("task");
  const [snapshot, setSnapshot] = useState<BootstrapSnapshot | null>(null);
  const [bootstrapError, setBootstrapError] = useState<string | null>(null);
  const [projects, setProjects] = useState<DesktopProject[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [tasks, setTasks] = useState<DesktopTask[]>([]);
  const [taskDetail, setTaskDetail] = useState<TaskDetail | null>(null);
  const [taskError, setTaskError] = useState<string | null>(null);
  const [showProjectForm, setShowProjectForm] = useState(false);
  const [pendingApprovals, setPendingApprovals] = useState<ToolApproval[]>([]);
  const [workspaceFiles, setWorkspaceFiles] = useState<WorkspaceEntry[]>([]);
  const [workspaceRead, setWorkspaceRead] = useState<WorkspaceRead | null>(null);
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>("system");
  const [query, setQuery] = useState("Qwen");
  const [models, setModels] = useState<ModelSearchResult[]>([]);
  const [selected, setSelected] = useState<ModelSearchResult | null>(null);
  const [artifactName, setArtifactName] = useState<string | null>(null);
  const [assessment, setAssessment] = useState<ModelAssessment | null>(null);
  const [placement, setPlacement] = useState<PlacementPlan | null>(null);
  const [searchState, setSearchState] = useState<"idle" | "loading" | "error">("idle");
  const [searchError, setSearchError] = useState<string | null>(null);
  const [defaultSaved, setDefaultSaved] = useState(false);
  const [browserAddress, setBrowserAddress] = useState("http://127.0.0.1:4173");
  const [browserSrc, setBrowserSrc] = useState("about:blank");
  const [taskRun, setTaskRun] = useState<TaskRun | null>(null);
  const activeHarness = useRef<PiHarnessInstance | null>(null);
  const workspaceRef = useRef<HTMLElement | null>(null);
  const taskCancelled = useRef(false);
  const [downloadState, setDownloadState] = useState<{
    state: "idle" | "running" | "done" | "error";
    message?: string;
  }>({ state: "idle" });
  const [downloadProgress, setDownloadProgress] = useState<DownloadProgress | null>(null);
  const [downloads, setDownloads] = useState<DownloadedModel[]>([]);
  const [downloadsError, setDownloadsError] = useState<string | null>(null);
  const [importState, setImportState] = useState<{ state: "idle" | "running" | "done" | "error"; message?: string }>({ state: "idle" });
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
  const [runtimeControl, setRuntimeControl] = useState<{ state: "idle" | "running" | "error"; message?: string }>({ state: "idle" });
  const [rendererMetrics, setRendererMetrics] = useState<RendererMetric[]>([]);

  useEffect(() => {
    let active = true;
    Promise.allSettled([desktop.bootstrap(), desktop.listDownloads(), desktop.listProjects()]).then(
      ([bootstrap, downloaded, knownProjects]) => {
        if (!active) return;
        if (bootstrap.status === "fulfilled") setSnapshot(bootstrap.value);
        else setBootstrapError(errorMessage(bootstrap.reason));
        if (downloaded.status === "fulfilled") setDownloads(downloaded.value);
        else setDownloadsError(errorMessage(downloaded.reason));
        if (knownProjects.status === "fulfilled") {
          setProjects(knownProjects.value);
          setSelectedProjectId((current) => current ?? knownProjects.value[0]?.id ?? null);
        } else {
          setTaskError(errorMessage(knownProjects.reason));
        }
        if (!performance.getEntriesByName("alpine:bootstrap").length) {
          if (!performance.getEntriesByName("alpine:renderer:start").length) performance.mark("alpine:renderer:start");
          performance.mark("alpine:bootstrap:ready");
          performance.measure("alpine:bootstrap", "alpine:renderer:start", "alpine:bootstrap:ready");
        }
        setRendererMetrics(collectRendererMetrics(bootstrap.status === "fulfilled" && bootstrap.value.settings.localMetricsEnabled));
      },
    );
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
        setEvaluationState((current) => ({ ...current, state: progress.state === "completed" ? "done" : progress.state === "failed" ? "error" : "running", scope: progress.scope, progress, error: progress.state === "failed" ? progress.message : current.error })),
      )
      .then((next) => { unlisten = next; })
      .catch(() => undefined);
    return () => unlisten?.();
  }, [desktop]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    desktop
      .subscribeDownloadProgress((progress) => setDownloadProgress(progress))
      .then((next) => { unlisten = next; })
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

  const selectedProject = projects.find((project) => project.id === selectedProjectId) ?? null;
  const selectedArtifact =
    selected?.artifacts.find((artifact) => artifact.filename === artifactName) ?? selected?.artifacts[0];
  const selectedInstalled = Boolean(selected && selectedArtifact && selected.revision && downloads.some((model) =>
    model.state === "installed"
      && model.repoId === selected.id
      && model.revision === selected.revision
      && model.filename === selectedArtifact.filename,
  ));
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
    if (!snapshot?.settings.defaultModel) throw new Error("Choose a default model before creating a Task.");
    if (!selectedProjectId) throw new Error("Add a Selected Project before creating a Task.");
    if (taskDetail) {
      const updated = await desktop.setTaskStatus(taskDetail.task.id, "running");
      setTaskDetail({ ...taskDetail, task: updated });
      return { task: updated, history: taskDetail.messages };
    }
    const task = await desktop.createTask({
      projectId: selectedProjectId,
      title: taskTitle(prompt),
      modelRepoId: snapshot.settings.defaultModel.repoId,
      modelFilename: snapshot.settings.defaultModel.filename,
      profile: snapshot.settings.defaultProfile,
    });
    const running = await desktop.setTaskStatus(task.id, "running");
    setTaskDetail({ task: running, messages: [], events: [] });
    await refreshTasks(selectedProjectId);
    return { task: running, history: [] };
  };

  const runTask = async (prompt: string) => {
    taskCancelled.current = false;
    setTaskError(null);
    setTaskRun({ prompt, response: "", state: "running" });
    let response = "";
    let taskId: string | undefined;
    try {
      const { task, history } = await ensureTask(prompt);
      taskId = task.id;
      setTaskRun({ taskId, prompt, response: "", state: "running" });
      const measurePerformance = snapshot?.settings.localMetricsEnabled !== false;
      if (measurePerformance) performance.mark("alpine:pi-launch:start");
      const [launch, { PiHarness }] = await Promise.all([desktop.resolvePiLaunch(), import("./harness/pi")]);
      if (taskCancelled.current) {
        await desktop.setTaskStatus(taskId, "cancelled");
        setTaskRun({ taskId, prompt, response, state: "cancelled" });
        return;
      }
      const harness = new PiHarness(launch, {
        taskId,
        desktop,
        history,
        onApproval: (approval) =>
          setPendingApprovals((current) => [
            ...current.filter((candidate) => candidate.id !== approval.id),
            approval,
          ]),
      });
      activeHarness.current = harness;
      let observedFirstDelta = false;
      if (measurePerformance) {
        performance.mark("alpine:pi-launch:ready");
        performance.measure("alpine:pi-launch", "alpine:pi-launch:start", "alpine:pi-launch:ready");
      }
      harness.subscribe(async (event) => {
        if (event.type === "message_update" && event.assistantMessageEvent.type === "text_delta") {
          if (measurePerformance && !observedFirstDelta) {
            observedFirstDelta = true;
            performance.mark("alpine:stream:first-event:ready");
            performance.measure("alpine:stream:first-event", "alpine:stream:start", "alpine:stream:first-event:ready");
          }
          response += event.assistantMessageEvent.delta;
          if (!taskCancelled.current) setTaskRun({ taskId, prompt, response, state: "running" });
          return;
        }
        await persistAgentEvent(
          desktop,
          taskId!,
          event,
          (message) => {
            setTaskDetail((current) => {
              if (!current || current.task.id !== taskId) return current;
              return { ...current, messages: [...current.messages, message] };
            });
          },
          (persisted) => {
            setTaskDetail((current) => {
              if (!current || current.task.id !== taskId) return current;
              return { ...current, events: [...current.events, persisted] };
            });
            if (persisted.kind === "tool.finished") {
              const payload = persisted.payload as { toolName?: string };
              if (payload.toolName === "edit_file") setInspectorTab("changes");
              if (payload.toolName === "run_command") setInspectorTab("terminal");
            }
          },
        );
      });
      if (measurePerformance) performance.mark("alpine:stream:start");
      await harness.prompt(prompt);
      if (measurePerformance) {
        performance.mark("alpine:stream:end");
        performance.measure("alpine:stream:duration", "alpine:stream:start", "alpine:stream:end");
      }
      if (harness.agent.state.errorMessage) throw new Error(harness.agent.state.errorMessage);
      const status = taskCancelled.current ? "cancelled" : "completed";
      await desktop.setTaskStatus(taskId, status);
      setTaskRun({ taskId, prompt, response, state: taskCancelled.current ? "cancelled" : "done" });
    } catch (error) {
      const message = errorMessage(error);
      if (taskId) {
        const status = taskCancelled.current ? "cancelled" : "failed";
        await desktop.setTaskStatus(taskId, status, taskCancelled.current ? null : message).catch(() => undefined);
      }
      setTaskRun({ taskId, prompt, response, state: taskCancelled.current ? "cancelled" : "error", error: taskCancelled.current ? undefined : message });
    } finally {
      activeHarness.current = null;
      if (taskId) await refreshTask(taskId).catch((error) => setTaskError(errorMessage(error)));
    }
  };

  const steerTask = (text: string, mode: "steer" | "follow-up") => {
    const harness = activeHarness.current;
    if (!harness) return;
    if (mode === "steer") harness.steer(text);
    else harness.followUp(text);
    setTaskRun((current) => current ? { ...current, note: mode === "steer" ? "Direction queued for this run" : "Follow-up queued" } : current);
  };

  const cancelTask = () => {
    taskCancelled.current = true;
    activeHarness.current?.abort();
    if (taskDetail) void desktop.setTaskStatus(taskDetail.task.id, "cancelling");
    setTaskRun((current) => (current ? { ...current, state: "cancelling", note: undefined } : current));
  };

  const decideApproval = async (approval: ToolApproval, approved: boolean) => {
    const settled = await desktop.decideToolApproval(approval.id, approved);
    setPendingApprovals((current) => current.filter((candidate) => candidate.id !== settled.id));
    if (taskDetail) {
      const persisted = await desktop.appendTaskEvent({
        taskId: taskDetail.task.id,
        kind: "approval.decided",
        payload: { approvalId: settled.id, operation: settled.operation, approved },
      });
      setTaskDetail((current) => current ? { ...current, events: [...current.events, persisted] } : current);
    }
  };

  const openWorkspaceFile = async (path: string) => {
    if (!taskDetail) return;
    setInspectorTab("files");
    setWorkspaceRead(await desktop.readProjectFile(taskDetail.task.id, path));
  };

  const search = async (event: FormEvent) => {
    event.preventDefault();
    if (!query.trim()) return;
    setSearchState("loading");
    setSearchError(null);
    setSelected(null);
    setArtifactName(null);
    setAssessment(null);
    setPlacement(null);
    setDefaultSaved(false);
    try {
      setModels(await desktop.searchModels(query.trim()));
      setSearchState("idle");
    } catch (error) {
      setSearchError(errorMessage(error));
      setSearchState("error");
    }
  };

  const chooseModel = async (model: ModelSearchResult) => {
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
      setAssessment(nextAssessment);
      setPlacement(nextPlacement);
    } catch (error) {
      setSearchError(errorMessage(error));
    }
  };

  const chooseArtifact = async (filename: string) => {
    setArtifactName(filename);
    setDefaultSaved(false);
    const artifact = selected?.artifacts.find((candidate) => candidate.filename === filename);
    if (!artifact?.sizeBytes) {
      setAssessment(null);
      setPlacement(null);
      return;
    }
    const [nextAssessment, nextPlacement] = await Promise.all([
      desktop.assessModel(artifact.sizeBytes),
      desktop.planModelPlacement(artifact.sizeBytes),
    ]);
    setAssessment(nextAssessment);
    setPlacement(nextPlacement);
  };

  const saveDefault = async () => {
    if (!selected || !selectedArtifact) return;
    const settings = await desktop.setDefaultModel({ repoId: selected.id, filename: selectedArtifact.filename });
    setSnapshot((current) => (current ? { ...current, settings } : current));
    setDefaultSaved(true);
  };

  const downloadSelected = async () => {
    if (!selected || !selectedArtifact) return;
    if (!selected.revision) {
      setDownloadState({ state: "error", message: "Hugging Face did not return an exact repository revision; Alpine refused a mutable download." });
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
      setDownloadState({ state: "done", message: receipt.alreadyPresent ? "Already installed" : `Saved ${formatBytes(receipt.bytesWritten)}` });
    } catch (error) {
      setDownloadState({ state: "error", message: errorMessage(error) });
    }
  };

  const importModel = async (sourcePath: string) => {
    setImportState({ state: "running" });
    try {
      const model = await desktop.importModel(sourcePath);
      setDownloads(await desktop.listDownloads());
      setImportState({ state: "done", message: `Imported and verified ${model.filename}` });
    } catch (error) {
      setImportState({ state: "error", message: errorMessage(error) });
    }
  };

  const cancelSelectedDownload = async () => {
    if (!selected || !selectedArtifact) return;
    if (await desktop.cancelDownload({ repoId: selected.id, filename: selectedArtifact.filename })) {
      setDownloadState({ state: "running", message: "Cancelling after the current chunk…" });
    }
  };

  const saveSettings = async (update: SettingsUpdate) => {
    await desktop.updateSettings(update);
    setSnapshot(await desktop.bootstrap());
  };

  const useActiveRuntimeModel = async () => {
    if (!snapshot?.runtime.model) return;
    const settings = await desktop.setDefaultModel({ repoId: "local/alpine-install", filename: snapshot.runtime.model });
    setSnapshot({ ...snapshot, settings });
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
    setRuntimeControl({ state: "running", message: action === "start" ? "Starting verified session…" : "Stopping verified session…" });
    try {
      const runtime = action === "start" ? await desktop.startRuntime() : await desktop.stopRuntime();
      setSnapshot((current) => current ? { ...current, runtime } : current);
      setRuntimeControl({ state: "idle", message: runtime.detail });
    } catch (error) {
      setRuntimeControl({ state: "error", message: errorMessage(error) });
    }
  };

  const navigate = (next: View) => {
    setView(next);
    if (next === "browser") setInspectorTab("browser");
  };

  return (
    <div className="app-shell">
      <aside className="rail" aria-label="Primary navigation">
        <div className="brand"><span className="brand-mark"><Sparkle size={15} weight="fill" /></span><span>Alpine</span></div>
        <button className="new-task" type="button" onClick={startNewTask}><Plus size={16} />New task</button>
        <nav className="nav-list">
          <NavButton active={view === "task"} label="Home" onClick={() => navigate("task")}><House size={17} /></NavButton>
          <NavButton active={view === "models"} label="Models" onClick={() => navigate("models")}><HardDrives size={17} /></NavButton>
          <NavButton active={view === "downloads"} label="Downloads" onClick={() => navigate("downloads")}><CloudArrowDown size={17} /></NavButton>
          <NavButton active={view === "analysis"} label="Analysis" onClick={() => navigate("analysis")}><Gauge size={17} /></NavButton>
          <NavButton active={view === "browser"} label="Browser" onClick={() => navigate("browser")}><Browser size={17} /></NavButton>
        </nav>
        <div className="project-switcher"><div><span>Selected project</span><button type="button" aria-label="Add project" onClick={() => setShowProjectForm(true)}><Plus size={13} /></button></div><select aria-label="Selected project" value={selectedProjectId ?? ""} onChange={(event) => { setSelectedProjectId(event.target.value || null); startNewTask(); }}><option value="">No project selected</option>{projects.map((project) => <option key={project.id} value={project.id}>{project.name}</option>)}</select></div>
        <div className="rail-section"><p>Recent tasks</p>{tasks.length ? tasks.slice(0, 8).map((task) => <button key={task.id} type="button" className={taskDetail?.task.id === task.id ? "selected" : ""} onClick={() => void openTask(task.id)}><span>{task.title}</span><small>{task.status}</small></button>) : <span className="rail-empty">No tasks in this project</span>}</div>
        <div className="rail-spacer" />
        <div className="runtime-pill"><span className="status-dot" />Pi runtime · experimental</div>
        <button className="settings-link" type="button" onClick={() => navigate("settings")}><GearSix size={17} />Settings</button>
      </aside>

      <main className="workspace" ref={workspaceRef}>
        <header className="topbar"><div><strong>{taskDetail && view === "task" ? taskDetail.task.title : titleFor(view)}</strong><span>{selectedProject?.root ?? "No Selected Project"}</span></div><div className="top-actions"><button type="button" onClick={() => { setRendererMetrics(collectRendererMetrics(snapshot?.settings.localMetricsEnabled !== false)); setInspectorTab("system"); }}><Pulse size={16} />Metrics</button><button className="task-status" type="button" disabled>{taskDetail?.task.status ?? "new"}</button></div></header>
        {showProjectForm && <ProjectForm onClose={() => setShowProjectForm(false)} onCreate={createProject} />}
        {view === "task" && <TaskView snapshot={snapshot} bootstrapError={bootstrapError} project={selectedProject} detail={taskDetail} taskError={taskError} approvals={pendingApprovals} taskRun={taskRun} onExplore={() => navigate("models")} onAddProject={() => setShowProjectForm(true)} onRun={runTask} onQueue={steerTask} onCancel={cancelTask} onApproval={decideApproval} />}
        {view === "models" && <ModelsView query={query} setQuery={setQuery} search={search} state={searchState} error={searchError} models={models} selected={selected} chooseModel={chooseModel} />}
        {view === "browser" && <BrowserView address={browserAddress} setAddress={setBrowserAddress} onOpen={(address) => { setBrowserSrc(address); setInspectorTab("browser"); }} />}
        {view === "settings" && <SettingsView snapshot={snapshot} runtimeControl={runtimeControl} onRuntimeControl={controlRuntime} onSave={saveSettings} onUseActive={useActiveRuntimeModel} />}
        {view === "downloads" && <DownloadsView downloads={downloads} error={downloadsError} importState={importState} onImport={importModel} />}
        {view === "analysis" && <AnalysisView snapshot={snapshot} state={probeState} evaluation={evaluationState} onRun={runProbe} onEvaluate={runFullEvaluation} />}
      </main>

      <ContextInspector tab={inspectorTab} setTab={setInspectorTab} snapshot={snapshot} bootstrapError={bootstrapError} selected={selected} selectedArtifact={selectedArtifact} selectedInstalled={selectedInstalled} assessment={assessment} placement={placement} metrics={rendererMetrics} defaultSaved={defaultSaved} saveDefault={saveDefault} chooseArtifact={chooseArtifact} downloadState={downloadState} downloadProgress={downloadProgress} downloadSelected={downloadSelected} cancelDownload={cancelSelectedDownload} taskDetail={taskDetail} files={workspaceFiles} workspaceRead={workspaceRead} clearWorkspaceRead={() => setWorkspaceRead(null)} openFile={openWorkspaceFile} browserSrc={browserSrc} hardwareLine={hardwareLine} />
    </div>
  );
}

function NavButton({ active, label, onClick, children }: { active: boolean; label: string; onClick: () => void; children: React.ReactNode }) {
  return <button type="button" className={active ? "active" : ""} aria-current={active ? "page" : undefined} onClick={onClick}>{children}<span>{label}</span></button>;
}

function ProjectForm({ onClose, onCreate }: { onClose: () => void; onCreate: (name: string, root: string) => Promise<void> }) {
  const [name, setName] = useState("");
  const [root, setRoot] = useState("");
  const [state, setState] = useState<{ saving: boolean; error?: string }>({ saving: false });
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setState({ saving: true });
    try { await onCreate(name.trim(), root.trim()); }
    catch (error) { setState({ saving: false, error: errorMessage(error) }); }
  };
  return <div className="project-form-layer"><form className="project-form" onSubmit={submit}><div><span><Folder size={17} />Add Selected Project</span><button type="button" aria-label="Close project form" onClick={onClose}><X size={16} /></button></div><p>Alpine will scope file, search, edit, and command capabilities to this canonical repository root.</p><label>Project name<input aria-label="Project name" value={name} onChange={(event) => setName(event.target.value)} placeholder="Alpine" required /></label><label>Absolute project root<input aria-label="Absolute project root" value={root} onChange={(event) => setRoot(event.target.value)} placeholder="C:\\workspace\\project" required /></label>{state.error && <p className="error-banner">{state.error}</p>}<button className="primary-button" type="submit" disabled={state.saving}>{state.saving ? "Adding…" : "Add project"}</button></form></div>;
}

function TaskView({ snapshot, bootstrapError, project, detail, taskError, approvals, taskRun, onExplore, onAddProject, onRun, onQueue, onCancel, onApproval }: {
  snapshot: BootstrapSnapshot | null;
  bootstrapError: string | null;
  project: DesktopProject | null;
  detail: TaskDetail | null;
  taskError: string | null;
  approvals: ToolApproval[];
  taskRun: TaskRun | null;
  onExplore: () => void;
  onAddProject: () => void;
  onRun: (prompt: string) => Promise<void>;
  onQueue: (prompt: string, mode: "steer" | "follow-up") => void;
  onCancel: () => void;
  onApproval: (approval: ToolApproval, approved: boolean) => Promise<void>;
}) {
  const [draft, setDraft] = useState("");
  const running = taskRun?.state === "running" || taskRun?.state === "cancelling";
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const prompt = draft.trim();
    if (!prompt || taskRun?.state === "cancelling") return;
    setDraft("");
    if (running) onQueue(prompt, "steer");
    else void onRun(prompt);
  };
  const messages = detail?.messages ?? [];
  const streamingVisible = running && Boolean(taskRun?.response) && messages.at(-1)?.content !== taskRun?.response;
  return <div className="task-view"><div className={`task-stream ${detail || taskRun ? "has-run" : ""}`}>{messages.length ? <div className="transcript">{messages.map((message) => message.role === "user" ? <article className="user-message" key={message.id}><span>You</span><p>{message.content}</p></article> : <article className="assistant-message" key={message.id}><span><Sparkle size={13} weight="fill" />Alpine · Pi</span><p>{message.content}</p></article>)}{streamingVisible && <article className="assistant-message streaming"><span><CircleNotch className="spin" size={13} />Alpine · Pi</span><p>{taskRun?.response}</p></article>}</div> : taskRun ? <div className="transcript"><article className="user-message"><span>You</span><p>{taskRun.prompt}</p></article><article className="assistant-message"><span><CircleNotch className="spin" size={13} />Alpine · Pi</span>{taskRun.state === "error" ? <div className="error-banner">{taskRun.error}</div> : <p>{taskRun.response || (taskRun.state === "cancelled" ? "Task cancelled." : taskRun.state === "cancelling" ? "Cancelling the local task…" : "Starting the local harness…")}</p>}</article></div> : <><div className="task-kicker"><span className="status-dot" />{project ? "Selected Project ready" : "Project required"}</div><h1>What should we build locally?</h1><p>Alpine restores durable tasks, starts the selected local model through Pi, and keeps every file or command operation inside an explicit project boundary.</p><div className="system-summary"><Cpu size={19} /><div><strong>{snapshot?.hardware.cpu ?? (bootstrapError ? "Hardware scan unavailable" : "Inspecting your machine")}</strong><span>{snapshot ? `${formatBytes(snapshot.hardware.memoryBytes)} memory · ${snapshot.hardware.gpu ?? "CPU-only"}` : bootstrapError ?? "Collecting CPU, GPU, memory, drivers, and runtime availability."}</span></div></div><div className="empty-actions">{project ? <button className="secondary-button explore" type="button" onClick={onExplore}>Explore models<ArrowDown size={15} /></button> : <button className="primary-button" type="button" onClick={onAddProject}><Plus size={15} />Add Selected Project</button>}</div></>}</div>{taskError && <div className="inline-task-error"><WarningCircle size={16} />{taskError}</div>}{approvals.map((approval) => <ApprovalCard key={approval.id} approval={approval} onDecision={onApproval} />)}{taskRun?.note && <div className="queue-note"><Check size={14} />{taskRun.note}</div>}<form className="composer" onSubmit={submit}><textarea aria-label="Task prompt" value={draft} onChange={(event) => setDraft(event.target.value)} placeholder={running ? "Steer the running task…" : "Ask Alpine to inspect, change, test, or analyze this project…"} /><div className="composer-footer"><div className="composer-context"><button type="button" onClick={onAddProject}><Plus size={16} />{project?.name ?? "Add project"}</button>{running && <button type="button" disabled={!draft.trim()} onClick={() => { const prompt = draft.trim(); if (!prompt) return; setDraft(""); onQueue(prompt, "follow-up"); }}>Queue follow-up</button>}</div><div><button type="button" disabled>Pi 0.84.2</button><button type="button" disabled>{snapshot?.settings.defaultModel?.filename ?? "Choose model"}</button><button className="send" type={running && !draft.trim() ? "button" : "submit"} aria-label={running && !draft.trim() ? "Cancel task" : running ? "Steer task" : "Run task"} disabled={taskRun?.state === "cancelling" || (!running && (!draft.trim() || !project))} onClick={running && !draft.trim() ? onCancel : undefined}>{running && !draft.trim() ? <StopCircle size={17} weight="fill" /> : <ArrowDown size={17} weight="bold" />}</button></div></div></form></div>;
}

function ApprovalCard({ approval, onDecision }: { approval: ToolApproval; onDecision: (approval: ToolApproval, approved: boolean) => Promise<void> }) {
  const [deciding, setDeciding] = useState(false);
  const primary = approval.operation === "shell" ? String(approval.proposal.command ?? "") : String(approval.proposal.path ?? "");
  const decide = async (approved: boolean) => {
    setDeciding(true);
    try { await onDecision(approval, approved); }
    finally { setDeciding(false); }
  };
  return <section className="approval-card"><div><ShieldCheck size={18} /><span><strong>{approval.operation === "shell" ? "Run command?" : "Apply exact edit?"}</strong><small>Pi is paused until you decide</small></span></div><code>{primary}</code>{approval.operation === "edit" && <p>{String(approval.proposal.oldText ?? "").slice(0, 140)} → {String(approval.proposal.newText ?? "").slice(0, 140)}</p>}<div><button type="button" disabled={deciding} onClick={() => void decide(false)}>Deny</button><button className="primary-button" type="button" disabled={deciding} onClick={() => void decide(true)}>{deciding ? "Deciding…" : "Approve once"}</button></div></section>;
}

function ModelsView({ query, setQuery, search, state, error, models, selected, chooseModel }: { query: string; setQuery: (value: string) => void; search: (event: FormEvent) => void; state: "idle" | "loading" | "error"; error: string | null; models: ModelSearchResult[]; selected: ModelSearchResult | null; chooseModel: (model: ModelSearchResult) => void }) {
  return <div className="content-view"><div className="page-heading"><p className="eyebrow">Hugging Face catalog</p><h1>Find a model for this machine</h1><p>Search GGUF repositories, inspect exact artifacts, then estimate fit before committing disk space.</p></div><form className="model-search" onSubmit={search}><MagnifyingGlass size={18} /><input type="search" aria-label="Search Hugging Face" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search models, publishers, or architectures" /><button type="submit" disabled={state === "loading"}>{state === "loading" ? "Searching…" : "Search"}</button></form>{error && <div className="error-banner"><WarningCircle size={16} />{error}</div>}<div className="model-results">{models.map((model) => <button type="button" key={model.id} className={selected?.id === model.id ? "selected" : ""} onClick={() => void chooseModel(model)}><div className="model-icon"><Sparkle size={17} /></div><div><strong>{model.id}</strong><span>{model.publisher} · {model.artifacts.length} GGUF {model.artifacts.length === 1 ? "file" : "files"} · updated {formatDate(model.lastModified)} · rev {model.revision?.slice(0, 8) ?? "unresolved"}</span></div><div className="model-stats"><span>{formatCount(model.downloads)} downloads</span><small>{formatCount(model.likes)} likes{model.gated ? " · gated" : ""}</small></div></button>)}</div>{state === "idle" && !models.length && <div className="empty-list"><HardDrives size={24} /><p>Search Hugging Face to inspect exact downloadable artifacts.</p></div>}</div>;
}

function BrowserView({ address, setAddress, onOpen }: { address: string; setAddress: (value: string) => void; onOpen: (address: string) => void }) {
  const [error, setError] = useState<string | null>(null);
  const open = (event: FormEvent) => {
    event.preventDefault();
    try {
      const url = new URL(address);
      if (!/^https?:$/.test(url.protocol)) throw new Error("Only HTTP and HTTPS previews are supported.");
      if (!["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)) throw new Error("The embedded Browser Surface is limited to localhost. Open external or authenticated pages in an explicit full browser.");
      setError(null);
      onOpen(url.toString());
    } catch (cause) { setError(errorMessage(cause)); }
  };
  return <div className="content-view browser-view"><div className="page-heading"><p className="eyebrow">Local artifact browser</p><h1>Browser</h1><p>Open localhost previews in the contextual inspector. Authenticated and external browsing remains a separate authority choice.</p></div><form className="browser-bar" onSubmit={open}><input aria-label="Browser address" value={address} onChange={(event) => setAddress(event.target.value)} /><button type="submit">Open</button></form>{error && <div className="error-banner"><WarningCircle size={16} />{error}</div>}<section className="browser-guidance"><Browser size={24} /><div><strong>Preview opens in the right inspector</strong><p>Keep the task stream visible while inspecting the app or artifact it produced.</p></div></section></div>;
}

function DownloadsView({ downloads, error, importState, onImport }: { downloads: DownloadedModel[]; error: string | null; importState: { state: "idle" | "running" | "done" | "error"; message?: string }; onImport: (sourcePath: string) => Promise<void> }) {
  const [sourcePath, setSourcePath] = useState("");
  const submit = (event: FormEvent) => { event.preventDefault(); if (sourcePath.trim()) void onImport(sourcePath.trim()); };
  return <div className="content-view"><div className="page-heading"><p className="eyebrow">Local Model Registry</p><h1>Models on this machine</h1><p>Downloaded and imported GGUF artifacts share one verified, provenance-aware registry.</p></div><form className="import-model" onSubmit={submit}><input aria-label="Import GGUF path" value={sourcePath} onChange={(event) => setSourcePath(event.target.value)} placeholder="Absolute path to an existing .gguf" /><button type="submit" disabled={!sourcePath.trim() || importState.state === "running"}>{importState.state === "running" ? "Importing…" : "Import GGUF"}</button></form>{importState.message && <p className={importState.state === "error" ? "settings-message error" : "settings-message"}>{importState.message}</p>}{error && <div className="error-banner"><WarningCircle size={16} />{error}</div>}<div className="download-list">{downloads.map((model) => <div key={`${model.localPath ?? "scan"}:${model.filename}`}><div className="model-icon"><HardDrives size={17} /></div><div><strong>{model.filename}</strong><span>{formatBytes(model.sizeBytes)} · {model.source ?? "unregistered install file"}{model.revision ? ` · rev ${model.revision.slice(0, 8)}` : ""}{model.sha256 ? ` · sha256 ${model.sha256.slice(0, 10)}…` : ""}</span></div><span className={`download-state ${model.state}`}>{model.state}</span></div>)}{!downloads.length && !error && <div className="empty-list"><CloudArrowDown size={24} /><p>No local GGUF artifacts were found.</p></div>}</div></div>;
}

function AnalysisView({ snapshot, state, evaluation, onRun, onEvaluate }: { snapshot: BootstrapSnapshot | null; state: { state: "idle" | "running" | "done" | "error"; report?: RuntimeProbeReport; error?: string }; evaluation: { state: "idle" | "running" | "done" | "error"; scope?: EvaluationScope; progress?: EvaluationProgress; report?: FullEvaluationSummary; error?: string }; onRun: () => Promise<void>; onEvaluate: (scope: EvaluationScope) => Promise<void> }) {
  const [scope, setScope] = useState<EvaluationScope>("candidate");
  const selectedMatchesRuntime = Boolean(snapshot?.settings.defaultModel && snapshot.runtime.model && snapshot.settings.defaultModel.filename.toLowerCase() === snapshot.runtime.model.toLowerCase());
  const enabled = snapshot?.runtime.state !== "unconfigured" && snapshot?.runtime.state !== "unavailable" && selectedMatchesRuntime;
  const metrics = evaluation.report ? evaluationWorkloads(evaluation.report) : [];
  return <div className="content-view analysis-view"><div className="page-heading"><p className="eyebrow">Evidence, not guesses</p><h1>Analysis</h1><p>Measure conservative runtime health or run Alpine's bounded multi-Profile tuning and Qualification engine. Evaluation never changes the daily default.</p></div><div className="analysis-grid"><section className="analysis-card"><div><Gauge size={22} /><div><strong>Local runtime diagnostic</strong><p>Starts or reuses the configured llama.cpp session, requires exact output, and records end-to-end latency.</p></div></div><dl><div><dt>Model</dt><dd>{snapshot?.runtime.model ?? "Not configured"}</dd></div><div><dt>Profile</dt><dd>{snapshot?.runtime.profile ?? "Not configured"}</dd></div><div><dt>Evidence</dt><dd>Measured diagnostic — not Qualification</dd></div></dl>{!selectedMatchesRuntime && <p className="analysis-warning"><WarningCircle size={15} />Select the active runtime model in Settings, or configure the downloaded artifact in Alpine first.</p>}<button className="primary-button" type="button" disabled={!enabled || state.state === "running"} onClick={() => void onRun()}>{state.state === "running" ? <><CircleNotch className="spin" size={16} />Running diagnostic…</> : "Run measured diagnostic"}</button></section><section className="analysis-card full-evaluation-card"><div><Pulse size={22} /><div><strong>Bounded Profile evaluation</strong><p>Measures Stable and candidate Profiles, separates workload metrics, selects only when policy proves an improvement, and restores the prior Inference Session.</p></div></div><dl><div><dt>Scope</dt><dd><select aria-label="Evaluation scope" value={scope} onChange={(event) => setScope(event.target.value as EvaluationScope)}><option value="candidate">Candidate · tune + final Qualification</option><option value="validated">Validated · add stability, context, tool task</option><option value="production">Production · add rollback and operator gate</option></select></dd></div><div><dt>Profiles</dt><dd>stable-16k vs turbo-16k</dd></div><div><dt>Deployment</dt><dd>Never changed by evaluation</dd></div></dl><button className="primary-button" type="button" disabled={!enabled || evaluation.state === "running"} onClick={() => void onEvaluate(scope)}>{evaluation.state === "running" ? <><CircleNotch className="spin" size={16} />Running {evaluation.scope} evaluation…</> : "Run full analysis"}</button>{evaluation.progress && <p className="evaluation-progress-note">{evaluation.progress.message}</p>}</section></div>{state.report && <section className="probe-report"><p className="eyebrow">Diagnostic result</p><div><strong>{state.report.qualityPass ? "Exact output passed" : "Exact output failed"}</strong><span>{state.report.latencyMs} ms end-to-end · {state.report.outputTokens ?? "unknown"} output tokens</span></div><p>{state.report.evidenceLabel}</p></section>}{state.error && <div className="error-banner"><WarningCircle size={16} />{state.error}</div>}{evaluation.report && <section className="evaluation-report"><div className="evaluation-decision"><div><p className="eyebrow">{evaluation.report.scope} evaluation</p><h2>{evaluation.report.decision}</h2><p>{evaluation.report.recommendation}</p></div><span className={`decision-pill ${evaluation.report.decision}`}>{evaluation.report.selectedProfile ?? "retain baseline"}</span></div><div className="metric-grid">{metrics.map((metric) => <article key={metric.name}><strong>{metric.label}</strong><span>{metric.value}</span><small>{metric.detail}</small></article>)}</div><div className="gate-grid"><EvidenceGate title="Correctness" value={summaryFlag(evaluation.report, "all_quality_pass") ? "Passed" : "Not proven"} /><EvidenceGate title="Determinism" value={summaryFlag(evaluation.report, "all_deterministic") ? "Passed" : "Not proven"} /><EvidenceGate title="Peak VRAM" value={evaluationResourceMetric(evaluation.report, "vram_peak_mib")} /><EvidenceGate title="Shared-memory spill" value={evaluationResourceMetric(evaluation.report, "shared_memory_peak_mib")} /><EvidenceGate title="Same-process stability" value={evaluation.report.sameProcessRequests ? `${evaluation.report.sameProcessRequests} requests` : "Not run"} /><EvidenceGate title="Clean restarts" value={evaluation.report.cleanRestarts ? `${evaluation.report.cleanRestarts} / ${evaluation.report.cleanRestarts}` : "Not run"} /><EvidenceGate title="Near-limit context" value={evaluation.report.nearLimitContextTokens ? `${evaluation.report.nearLimitContextTokens} tokens` : "Not run"} /><EvidenceGate title="Tool task" value={evaluation.report.goldenToolCalls != null ? `${evaluation.report.goldenToolCalls} calls · ${evaluation.report.goldenToolFailures} recovered failures` : "Not run"} /><EvidenceGate title="Rollback" value={evaluation.report.rollbackProved ? `${evaluation.report.rollbackProfile} proved` : `${evaluation.report.rollbackProfile} preserved`} /><EvidenceGate title="Prior Session" value={evaluation.report.priorSessionRestored ? "Restored" : "Not proven"} /></div><footer><span>Plan {evaluation.report.planId} · sha256 {evaluation.report.planSha256.slice(0, 12)}…</span><code>{evaluation.report.artifactPath}</code></footer></section>}{evaluation.error && <div className="error-banner"><WarningCircle size={16} />{evaluation.error}</div>}</div>;
}

function EvidenceGate({ title, value }: { title: string; value: string }) { return <div><span>{title}</span><strong>{value}</strong></div>; }

function SettingsView({ snapshot, runtimeControl, onRuntimeControl, onSave, onUseActive }: { snapshot: BootstrapSnapshot | null; runtimeControl: { state: "idle" | "running" | "error"; message?: string }; onRuntimeControl: (action: "start" | "stop") => Promise<void>; onSave: (update: SettingsUpdate) => Promise<void>; onUseActive: () => Promise<void> }) {
  const [installRoot, setInstallRoot] = useState("");
  const [evaluationRoot, setEvaluationRoot] = useState("");
  const [profile, setProfile] = useState("stable-16k");
  const [localMetrics, setLocalMetrics] = useState(true);
  const [saveState, setSaveState] = useState<{ state: "idle" | "saving" | "done" | "error"; message?: string }>({ state: "idle" });
  useEffect(() => { if (snapshot) { setInstallRoot(snapshot.settings.installRoot); setEvaluationRoot(snapshot.settings.evaluationRepositoryRoot); setProfile(snapshot.settings.defaultProfile); setLocalMetrics(snapshot.settings.localMetricsEnabled); } }, [snapshot]);
  const save = async (event: FormEvent) => {
    event.preventDefault(); setSaveState({ state: "saving" });
    try { await onSave({ installRoot, evaluationRepositoryRoot: evaluationRoot, defaultProfile: profile, localMetricsEnabled: localMetrics }); setSaveState({ state: "done", message: "Settings saved and runtime rechecked." }); }
    catch (error) { setSaveState({ state: "error", message: errorMessage(error) }); }
  };
  const profiles = Array.from(new Set([profile, ...(snapshot?.runtime.availableProfiles ?? [])]));
  return <form className="content-view settings-view" onSubmit={save}><div className="page-heading"><p className="eyebrow">Workspace preferences</p><h1>Settings</h1><p>Configure the local model, runtime, storage, Agent Runtime, safety, appearance, and diagnostics.</p></div><section className="settings-group"><h2>Model & runtime</h2><SettingRow title="Default Agent Runtime" copy="Pi is a replaceable Adapter; Alpine retains Task and tool authority."><button type="button" disabled>Pi SDK 0.84.2 · experimental</button></SettingRow><SettingRow title="Default model" copy="New Tasks persist and launch this exact artifact. Restart not required."><div className="setting-action"><code>{snapshot?.settings.defaultModel?.filename ?? "Not selected"}</code>{snapshot?.runtime.model && <button type="button" onClick={() => void onUseActive()}>Use active model</button>}</div></SettingRow><SettingRow title="Default Profile" copy="The Alpine Profile started before Pi connects. Takes effect on the next Inference Session."><select aria-label="Default profile" value={profile} onChange={(event) => setProfile(event.target.value)}>{profiles.map((value) => <option key={value} value={value}>{value}</option>)}</select></SettingRow><SettingRow title="Runtime status" copy={runtimeControl.message ?? snapshot?.runtime.detail ?? "Inspecting the local control plane…"}><div className="runtime-controls"><span className={`settings-status ${runtimeControl.state === "error" ? "unavailable" : snapshot?.runtime.state ?? "loading"}`}>{runtimeControl.state === "running" ? "working" : snapshot?.runtime.state ?? "loading"}</span><button type="button" disabled={!snapshot || runtimeControl.state === "running" || snapshot.runtime.state === "running" || snapshot.runtime.state === "unconfigured" || snapshot.runtime.state === "unavailable"} onClick={() => void onRuntimeControl("start")}>Start</button><button type="button" disabled={!snapshot || runtimeControl.state === "running" || snapshot.runtime.state !== "running"} onClick={() => void onRuntimeControl("stop")}>Stop</button></div></SettingRow></section><section className="settings-group"><h2>Storage & evidence</h2><SettingRow title="Alpine installation root" copy="Must be absolute. Models are stored below its models directory; runtime configuration remains authoritative."><input aria-label="Alpine installation root" value={installRoot} onChange={(event) => setInstallRoot(event.target.value)} /></SettingRow><SettingRow title="Evaluation repository root" copy="Must contain the versioned Alpine config and benchmark resources. Invalid paths are rejected before saving."><input aria-label="Evaluation repository root" value={evaluationRoot} onChange={(event) => setEvaluationRoot(event.target.value)} /></SettingRow></section><section className="settings-group"><h2>Safety & appearance</h2><SettingRow title="Consequential workspace operations" copy="Exact edits and shell commands require one Tool Approval; read, list, and search remain project-scoped."><span className="settings-status configured">approve once</span></SettingRow><SettingRow title="Appearance" copy="Alpine follows the compact dark desktop theme. System theme switching is not yet enabled."><button type="button" disabled>Dark</button></SettingRow></section><section className="settings-group"><h2>Privacy & diagnostics</h2><SettingRow title="Local performance measurements" copy="Records timings and counts only. Prompts, credentials, and repository content are excluded. No restart required."><button className={`toggle ${localMetrics ? "on" : ""}`} type="button" role="switch" aria-checked={localMetrics} onClick={() => setLocalMetrics((value) => !value)}><span /></button></SettingRow></section>{saveState.message && <p className={saveState.state === "error" ? "settings-message error" : "settings-message"}>{saveState.message}</p>}<button className="primary-button settings-save" type="submit" disabled={!snapshot || saveState.state === "saving"}>{saveState.state === "saving" ? "Saving…" : "Save settings"}</button></form>;
}

function SettingRow({ title, copy, children }: { title: string; copy: string; children: React.ReactNode }) { return <div className="setting-row"><div><strong>{title}</strong><p>{copy}</p></div><div>{children}</div></div>; }

function ContextInspector({ tab, setTab, snapshot, bootstrapError, selected, selectedArtifact, selectedInstalled, assessment, placement, metrics, defaultSaved, saveDefault, chooseArtifact, downloadState, downloadProgress, downloadSelected, cancelDownload, taskDetail, files, workspaceRead, clearWorkspaceRead, openFile, browserSrc, hardwareLine }: {
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
  downloadState: { state: "idle" | "running" | "done" | "error"; message?: string };
  downloadProgress: DownloadProgress | null;
  downloadSelected: () => Promise<void>;
  cancelDownload: () => Promise<void>;
  taskDetail: TaskDetail | null;
  files: WorkspaceEntry[];
  workspaceRead: WorkspaceRead | null;
  clearWorkspaceRead: () => void;
  openFile: (path: string) => Promise<void>;
  browserSrc: string;
  hardwareLine: string;
}) {
  const lastDiff = latestToolDetails(taskDetail?.events ?? [], "edit_file") as { diff?: string; path?: string } | null;
  const lastShell = latestToolDetails(taskDetail?.events ?? [], "run_command") as { command?: string; stdout?: string; stderr?: string; exitCode?: number; durationMs?: number } | null;
  const tabs: Array<{ value: InspectorTab; label: string; icon: React.ReactNode }> = [
    { value: "system", label: "System", icon: <Cpu size={14} /> },
    { value: "files", label: "Files", icon: <FileText size={14} /> },
    { value: "changes", label: "Changes", icon: <GitDiff size={14} /> },
    { value: "terminal", label: "Terminal", icon: <TerminalWindow size={14} /> },
    { value: "browser", label: "Preview", icon: <Browser size={14} /> },
  ];
  return <aside className="inspector" aria-label="Context inspector"><div className="inspector-tabs">{tabs.map((item) => <button key={item.value} type="button" className={tab === item.value ? "active" : ""} aria-label={item.label} title={item.label} onClick={() => setTab(item.value)}>{item.icon}</button>)}</div><div className="inspector-header"><span>{tabs.find((item) => item.value === tab)?.label}</span><small>{taskDetail?.task.modelFilename ?? snapshot?.runtime.profile ?? "local"}</small></div><div className="inspector-content">{tab === "system" && <><section className="hardware-card"><div className="card-label"><Cpu size={15} />This machine</div>{snapshot ? <><strong>{snapshot.hardware.gpu ?? snapshot.hardware.cpu}</strong><dl><div><dt>VRAM</dt><dd>{formatBytes(snapshot.hardware.vramBytes)}</dd></div><div><dt>Memory</dt><dd>{formatBytes(snapshot.hardware.memoryBytes)}</dd></div><div><dt>Driver</dt><dd>{snapshot.hardware.driver ?? "Not reported"}</dd></div></dl></> : <p className="muted">{bootstrapError ?? "Reading local hardware…"}</p>}</section>{metrics.length > 0 && <section className="metrics-card"><p className="eyebrow">Local app metrics</p>{metrics.map((metric) => <div key={metric.label}><span>{metric.label}</span><strong>{metric.value}</strong><small>{metric.detail}</small></div>)}</section>}{selected ? <section className="selection-card"><p className="eyebrow">Selected artifact</p><strong>{selected.id}</strong>{selected.artifacts.length > 1 ? <select className="artifact-select" aria-label="Model artifact" value={selectedArtifact?.filename ?? ""} onChange={(event) => void chooseArtifact(event.target.value)}>{selected.artifacts.map((artifact) => <option key={artifact.filename} value={artifact.filename}>{artifact.filename} · {formatBytes(artifact.sizeBytes)}</option>)}</select> : <span>{selectedArtifact?.filename ?? "No GGUF artifact"}</span>}{assessment ? <div className={`fit-badge ${assessment.status === "unlikely-to-fit" ? "warning" : ""}`}>{assessment.status === "unlikely-to-fit" ? <WarningCircle size={14} /> : <Check size={14} />}{fitLabel(assessment)}</div> : <div className="fit-badge neutral">Size metadata required</div>}{placement && <div className="placement-plan"><p className="eyebrow">Placement estimate</p>{placement.candidates.map((candidate) => <div className={candidate.id === placement.recommendedId ? "recommended" : ""} key={candidate.id}><span>{candidate.label}</span><strong>{candidate.viable ? `${candidate.gpuResidencyPercent}% GPU` : "Does not fit safely"}</strong></div>)}<small>{placement.evidenceLabel}</small></div>}{assessment && <p className="evidence">{assessment.evidenceLabel}</p>}<button className="primary-button" type="button" onClick={() => void saveDefault()} disabled={!selectedArtifact || !selectedInstalled}>{defaultSaved ? <><Check size={16} />Default for new tasks</> : selectedInstalled ? "Set as default" : "Download before selecting"}</button><button className="secondary-button" type="button" onClick={() => void (downloadState.state === "running" ? cancelDownload() : downloadSelected())}>{downloadState.state === "running" ? <><CircleNotch className="spin" size={16} />Cancel download</> : <><DownloadSimple size={16} />Download model</>}</button>{downloadProgress && downloadState.state === "running" && <div className="download-progress"><span style={{ width: downloadProgress.totalBytes ? `${Math.min(100, (downloadProgress.bytesWritten / downloadProgress.totalBytes) * 100)}%` : "18%" }} /><small>{downloadProgress.state} · {formatBytes(downloadProgress.bytesWritten)} / {formatBytes(downloadProgress.totalBytes)}</small></div>}{downloadState.message && <p className={downloadState.state === "error" ? "download-note error" : "download-note"}>{downloadState.message}</p>}</section> : <section className="inspector-empty"><Code size={20} /><p>Select a model or open a Task to inspect its local context.</p></section>}</>}{tab === "files" && <section className="file-inspector">{workspaceRead ? <><button className="back-link" type="button" onClick={clearWorkspaceRead}><ArrowDown size={14} />Back to files</button><p className="file-path">{workspaceRead.path}</p><pre>{workspaceRead.content}</pre>{workspaceRead.truncated && <small>Showing {workspaceRead.startLine}–{workspaceRead.endLine} of {workspaceRead.totalLines}</small>}</> : files.length ? files.map((entry) => <button type="button" key={entry.path} disabled={entry.kind !== "file"} onClick={() => void openFile(entry.path)}>{entry.kind === "directory" ? <Folder size={14} /> : <FileText size={14} />}<span>{entry.path}</span>{entry.kind === "file" && <small>{formatCompactBytes(entry.sizeBytes)}</small>}</button>) : <div className="inspector-empty"><Folder size={20} /><p>Open a durable Task to list its Selected Project.</p></div>}</section>}{tab === "changes" && <section className="artifact-panel">{lastDiff?.diff ? <><p className="eyebrow">Latest exact edit</p><pre className="diff-output">{lastDiff.diff}</pre></> : <div className="inspector-empty"><GitDiff size={20} /><p>Approved edits and their diffs appear here.</p></div>}</section>}{tab === "terminal" && <section className="artifact-panel">{lastShell ? <><p className="eyebrow">Approved command</p><code>{lastShell.command}</code><pre className="terminal-output">{lastShell.stdout}{lastShell.stderr ? `\n[stderr]\n${lastShell.stderr}` : ""}</pre><small>Exit {lastShell.exitCode} · {lastShell.durationMs} ms</small></> : <div className="inspector-empty"><TerminalWindow size={20} /><p>Approved tests and commands appear here with their exit status.</p></div>}</section>}{tab === "browser" && <section className="browser-inspector">{browserSrc === "about:blank" ? <div className="inspector-empty"><Browser size={20} /><p>Open a localhost address from Browser.</p></div> : <iframe title="Browser preview" src={browserSrc} sandbox="allow-forms allow-modals allow-pointer-lock allow-popups allow-same-origin allow-scripts" />}</section>}</div><div className="inspector-footer"><section className="runtime-card"><span className={`status-dot ${snapshot?.runtime.state === "unconfigured" || snapshot?.runtime.state === "unavailable" ? "warning" : ""}`} /><div><strong>{snapshot ? `Control plane ${snapshot.runtime.state}` : "Inspecting control plane"}</strong><small>{snapshot?.runtime.profile ?? "Waiting for profile"}</small></div></section><p className="hardware-line">{hardwareLine}</p></div></aside>;
}

async function persistAgentEvent(desktop: DesktopClient, taskId: string, event: AgentEvent, onMessage: (message: Awaited<ReturnType<DesktopClient["appendTaskMessage"]>>) => void, onEvent: (event: TaskEvent) => void) {
  if (event.type === "message_end" && (event.message.role === "user" || event.message.role === "assistant")) {
    const content = agentMessageText(event.message);
    if (content) onMessage(await desktop.appendTaskMessage({ taskId, role: event.message.role, content }));
  }
  const normalized = normalizeAgentEvent(event);
  if (normalized) onEvent(await desktop.appendTaskEvent({ taskId, ...normalized }));
}

function normalizeAgentEvent(event: AgentEvent): { kind: string; payload: unknown } | null {
  switch (event.type) {
    case "agent_start": return { kind: "agent.started", payload: {} };
    case "agent_end": return { kind: "agent.finished", payload: { messageCount: event.messages.length } };
    case "turn_start": return { kind: "turn.started", payload: {} };
    case "turn_end": return { kind: "turn.finished", payload: { toolResultCount: event.toolResults.length } };
    case "message_start": return { kind: "message.started", payload: { role: event.message.role } };
    case "message_end": return event.message.role === "assistant" ? { kind: "message.finished", payload: { role: "assistant", stopReason: event.message.stopReason, usage: event.message.usage, error: event.message.errorMessage ?? null } } : { kind: "message.finished", payload: { role: event.message.role } };
    case "tool_execution_start": return { kind: "tool.started", payload: { toolCallId: event.toolCallId, toolName: event.toolName, args: boundedPayload(event.args) } };
    case "tool_execution_update": return { kind: "tool.updated", payload: { toolCallId: event.toolCallId, toolName: event.toolName, details: boundedPayload(event.partialResult?.details ?? null) } };
    case "tool_execution_end": return { kind: "tool.finished", payload: { toolCallId: event.toolCallId, toolName: event.toolName, isError: event.isError, details: boundedPayload(event.result?.details ?? null) } };
    case "message_update": return null;
  }
}

function boundedPayload(value: unknown): unknown {
  const encoded = JSON.stringify(value);
  if (!encoded || encoded.length <= 256_000) return value;
  return { truncated: true, preview: encoded.slice(0, 256_000) };
}

function agentMessageText(message: AgentMessage): string {
  if (message.role === "user") {
    if (typeof message.content === "string") return message.content;
    return message.content.filter((part) => part.type === "text").map((part) => part.text).join("\n");
  }
  if (message.role === "assistant") return message.content.filter((part) => part.type === "text").map((part) => part.text).join("");
  return "";
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

function evaluationSummary(report: FullEvaluationSummary): Record<string, unknown> {
  const evidence = report.finalEvidence ?? {};
  return (evidence.result_summary as Record<string, unknown> | undefined)
    ?? (evidence.resultSummary as Record<string, unknown> | undefined)
    ?? {};
}

function summaryFlag(report: FullEvaluationSummary, key: string) {
  return evaluationSummary(report)[key] === true;
}

function evaluationResourceMetric(report: FullEvaluationSummary, key: string) {
  const resources = evaluationSummary(report).resources as Record<string, unknown> | undefined;
  const value = resources?.[key];
  return typeof value === "number" ? `${value.toFixed(0)} MiB` : "Not captured";
}

function evaluationWorkloads(report: FullEvaluationSummary) {
  const workloads = (evaluationSummary(report).workloads as Record<string, Record<string, unknown>> | undefined) ?? {};
  const definitions = [
    { name: "prefill-4k", label: "Prompt processing", metric: "prefill_tps" },
    { name: "novel-256", label: "Novel decode", metric: "decode_tps" },
    { name: "repeat-code-256", label: "Repeated-token decode", metric: "decode_tps" },
    { name: "structured-json-128", label: "Structured decode", metric: "decode_tps" },
  ];
  return definitions.map((definition) => {
    const workload = workloads[definition.name] ?? {};
    const metric = (workload[definition.metric] as Record<string, unknown> | undefined) ?? {};
    const median = typeof metric.median === "number" ? metric.median : null;
    const quality = typeof workload.quality_pass_rate === "number" ? workload.quality_pass_rate : null;
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

function errorMessage(error: unknown) { return error instanceof Error ? error.message : String(error); }
function titleFor(view: View) { return ({ task: "New task", models: "Models", downloads: "Downloads", analysis: "Analysis", browser: "Browser", settings: "Settings" } as const)[view]; }
