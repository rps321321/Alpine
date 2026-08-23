import {
  ArrowDown,
  Browser,
  CaretDown,
  Check,
  CircleNotch,
  CloudArrowDown,
  Cpu,
  DownloadSimple,
  Gauge,
  GearSix,
  HardDrives,
  House,
  MagnifyingGlass,
  Plus,
  Pulse,
  SlidersHorizontal,
  Sparkle,
  StopCircle,
  TerminalWindow,
  WarningCircle,
} from "@phosphor-icons/react";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import type { PiHarness as PiHarnessInstance } from "./harness/pi";
import type {
  BootstrapSnapshot,
  DesktopClient,
  DownloadedModel,
  ModelAssessment,
  ModelSearchResult,
  RuntimeProbeReport,
  SettingsUpdate,
} from "./desktop";

type View = "task" | "models" | "downloads" | "analysis" | "browser" | "settings";
type TaskRun = { prompt: string; response: string; state: "running" | "cancelling" | "done" | "cancelled" | "error"; error?: string };

const formatBytes = (bytes: number) => {
  if (!bytes) return "Size unavailable";
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
};

const formatCount = (value: number) =>
  new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value);

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
  const [query, setQuery] = useState("Qwen");
  const [models, setModels] = useState<ModelSearchResult[]>([]);
  const [selected, setSelected] = useState<ModelSearchResult | null>(null);
  const [artifactName, setArtifactName] = useState<string | null>(null);
  const [assessment, setAssessment] = useState<ModelAssessment | null>(null);
  const [searchState, setSearchState] = useState<"idle" | "loading" | "error">("idle");
  const [searchError, setSearchError] = useState<string | null>(null);
  const [defaultSaved, setDefaultSaved] = useState(false);
  const [browserAddress, setBrowserAddress] = useState("http://127.0.0.1:4173");
  const [taskRun, setTaskRun] = useState<TaskRun | null>(null);
  const activeHarness = useRef<PiHarnessInstance | null>(null);
  const taskCancelled = useRef(false);
  const [downloadState, setDownloadState] = useState<{ state: "idle" | "running" | "done" | "error"; message?: string }>({ state: "idle" });
  const [downloads, setDownloads] = useState<DownloadedModel[]>([]);
  const [downloadsError, setDownloadsError] = useState<string | null>(null);
  const [probeState, setProbeState] = useState<{ state: "idle" | "running" | "done" | "error"; report?: RuntimeProbeReport; error?: string }>({ state: "idle" });

  useEffect(() => {
    let active = true;
    desktop
      .bootstrap()
      .then((next) => active && setSnapshot(next))
      .catch((error: unknown) => {
        if (active) setBootstrapError(error instanceof Error ? error.message : String(error));
      });
    desktop
      .listDownloads()
      .then((next) => active && setDownloads(next))
      .catch((error: unknown) => {
        if (active) setDownloadsError(error instanceof Error ? error.message : String(error));
      });
    return () => {
      active = false;
    };
  }, [desktop]);

  const hardwareLine = useMemo(() => {
    if (!snapshot) return "Inspecting local hardware…";
    return `${snapshot.hardware.gpu ?? "CPU runtime"} · ${formatBytes(snapshot.hardware.vramBytes || snapshot.hardware.memoryBytes)}`;
  }, [snapshot]);
  const selectedArtifact = selected?.artifacts.find((artifact) => artifact.filename === artifactName) ?? selected?.artifacts[0];

  const search = async (event: FormEvent) => {
    event.preventDefault();
    if (!query.trim()) return;
    setSearchState("loading");
    setSearchError(null);
    setSelected(null);
    setArtifactName(null);
    setAssessment(null);
    setDefaultSaved(false);
    try {
      setModels(await desktop.searchModels(query.trim()));
      setSearchState("idle");
    } catch (error) {
      setSearchError(error instanceof Error ? error.message : String(error));
      setSearchState("error");
    }
  };

  const chooseModel = async (model: ModelSearchResult) => {
    setSelected(model);
    setArtifactName(model.artifacts[0]?.filename ?? null);
    setDefaultSaved(false);
    const artifact = model.artifacts[0];
    if (!artifact || !artifact.sizeBytes) {
      setAssessment(null);
      return;
    }
    try {
      setAssessment(await desktop.assessModel(artifact.sizeBytes));
    } catch (error) {
      setSearchError(error instanceof Error ? error.message : String(error));
    }
  };

  const chooseArtifact = async (filename: string) => {
    setArtifactName(filename);
    setDefaultSaved(false);
    const artifact = selected?.artifacts.find((candidate) => candidate.filename === filename);
    if (!artifact?.sizeBytes) {
      setAssessment(null);
      return;
    }
    setAssessment(await desktop.assessModel(artifact.sizeBytes));
  };

  const saveDefault = async () => {
    const artifact = selectedArtifact;
    if (!selected || !artifact) return;
    const settings = await desktop.setDefaultModel({ repoId: selected.id, filename: artifact.filename });
    setSnapshot((current) => (current ? { ...current, settings } : current));
    setDefaultSaved(true);
  };

  const runTask = async (prompt: string) => {
    taskCancelled.current = false;
    setTaskRun({ prompt, response: "", state: "running" });
    let response = "";
    try {
      const measurePerformance = snapshot?.settings.localMetricsEnabled !== false;
      if (measurePerformance) performance.mark("alpine:pi-launch:start");
      const [launch, { PiHarness }] = await Promise.all([
        desktop.resolvePiLaunch(),
        import("./harness/pi"),
      ]);
      if (taskCancelled.current) {
        setTaskRun({ prompt, response, state: "cancelled" });
        return;
      }
      const harness = new PiHarness(launch);
      activeHarness.current = harness;
      if (measurePerformance) {
        performance.mark("alpine:pi-launch:ready");
        performance.measure("alpine:pi-launch", "alpine:pi-launch:start", "alpine:pi-launch:ready");
      }
      harness.subscribe((event) => {
        if (event.type === "message_update" && event.assistantMessageEvent.type === "text_delta") {
          response += event.assistantMessageEvent.delta;
          if (!taskCancelled.current) setTaskRun({ prompt, response, state: "running" });
        }
      });
      await harness.prompt(prompt);
      setTaskRun({ prompt, response, state: taskCancelled.current ? "cancelled" : "done" });
    } catch (error) {
      if (taskCancelled.current) {
        setTaskRun({ prompt, response, state: "cancelled" });
      } else {
        setTaskRun({
          prompt,
          response,
          state: "error",
          error: error instanceof Error ? error.message : String(error),
        });
      }
    } finally {
      activeHarness.current = null;
    }
  };

  const cancelTask = () => {
    taskCancelled.current = true;
    activeHarness.current?.abort();
    setTaskRun((current) => current ? { ...current, state: "cancelling" } : current);
  };

  const downloadSelected = async () => {
    const artifact = selectedArtifact;
    if (!selected || !artifact) return;
    setDownloadState({ state: "running" });
    try {
      const receipt = await desktop.downloadModel(
        { repoId: selected.id, filename: artifact.filename },
        artifact.sizeBytes,
        artifact.sha256,
      );
      setDownloads(await desktop.listDownloads());
      setDownloadState({
        state: "done",
        message: receipt.alreadyPresent ? "Already installed" : `Saved ${formatBytes(receipt.bytesWritten)}`,
      });
    } catch (error) {
      setDownloadState({ state: "error", message: error instanceof Error ? error.message : String(error) });
    }
  };

  const cancelSelectedDownload = async () => {
    const artifact = selectedArtifact;
    if (!selected || !artifact) return;
    const cancelled = await desktop.cancelDownload({ repoId: selected.id, filename: artifact.filename });
    if (cancelled) setDownloadState({ state: "running", message: "Cancelling after the current chunk…" });
  };

  const saveSettings = async (update: SettingsUpdate) => {
    await desktop.updateSettings(update);
    const next = await desktop.bootstrap();
    setSnapshot(next);
  };

  const useActiveRuntimeModel = async () => {
    if (!snapshot?.runtime.model) return;
    const settings = await desktop.setDefaultModel({
      repoId: "local/alpine-install",
      filename: snapshot.runtime.model,
    });
    setSnapshot({ ...snapshot, settings });
  };

  const runProbe = async () => {
    setProbeState({ state: "running" });
    try {
      const report = await desktop.runRuntimeProbe();
      setProbeState({ state: "done", report });
      setSnapshot(await desktop.bootstrap());
    } catch (error) {
      setProbeState({ state: "error", error: error instanceof Error ? error.message : String(error) });
    }
  };

  return (
    <div className="app-shell">
      <aside className="rail" aria-label="Primary navigation">
        <div className="brand"><span className="brand-mark"><Sparkle size={15} weight="fill" /></span><span>Alpine</span></div>
        <button className="new-task" type="button" onClick={() => setView("task")}><Plus size={16} />New task</button>
        <nav className="nav-list">
          <NavButton active={view === "task"} label="Home" onClick={() => setView("task")}><House size={17} /></NavButton>
          <NavButton active={view === "models"} label="Models" onClick={() => setView("models")}><HardDrives size={17} /></NavButton>
          <NavButton active={view === "downloads"} label="Downloads" onClick={() => setView("downloads")}><CloudArrowDown size={17} /></NavButton>
          <NavButton active={view === "analysis"} label="Analysis" onClick={() => setView("analysis")}><Gauge size={17} /></NavButton>
          <NavButton active={view === "browser"} label="Browser" onClick={() => setView("browser")}><Browser size={17} /></NavButton>
        </nav>
        <div className="rail-section">
          <p>Recent tasks</p>
          <button type="button" onClick={() => setView("task")}><span>Profile local hardware</span><small>now</small></button>
          <button type="button" onClick={() => setView("models")}><span>Compare Qwen builds</span><small>draft</small></button>
        </div>
        <div className="rail-spacer" />
        <div className="runtime-pill"><span className="status-dot" />Pi runtime · experimental</div>
        <button className="settings-link" type="button" onClick={() => setView("settings")}><GearSix size={17} />Settings</button>
      </aside>

      <main className="workspace">
        <header className="topbar">
          <div><strong>{titleFor(view)}</strong><span>{view === "task" ? "Local workspace" : "Alpine Desktop"}</span></div>
          <div className="top-actions"><button type="button"><Pulse size={16} />Metrics</button><button className="icon-button" type="button" aria-label="More controls"><SlidersHorizontal size={17} /></button></div>
        </header>

        {view === "task" && <TaskView snapshot={snapshot} bootstrapError={bootstrapError} onExplore={() => setView("models")} taskRun={taskRun} onRun={runTask} onCancel={cancelTask} />}
        {view === "models" && (
          <ModelsView query={query} setQuery={setQuery} search={search} state={searchState} error={searchError} models={models} selected={selected} chooseModel={chooseModel} />
        )}
        {view === "browser" && <BrowserView address={browserAddress} setAddress={setBrowserAddress} />}
        {view === "settings" && <SettingsView snapshot={snapshot} onSave={saveSettings} onUseActive={useActiveRuntimeModel} />}
        {view === "downloads" && <DownloadsView downloads={downloads} error={downloadsError} />}
        {view === "analysis" && <AnalysisView snapshot={snapshot} state={probeState} onRun={runProbe} />}
      </main>

      <aside className="inspector" aria-label="Context inspector">
        <div className="inspector-header"><span>System context</span><button type="button" aria-label="Collapse inspector"><CaretDown size={15} /></button></div>
        <section className="hardware-card">
          <div className="card-label"><Cpu size={15} />This machine</div>
          {snapshot ? (
            <>
              <strong>{snapshot.hardware.gpu ?? snapshot.hardware.cpu}</strong>
              <dl><div><dt>VRAM</dt><dd>{formatBytes(snapshot.hardware.vramBytes)}</dd></div><div><dt>Memory</dt><dd>{formatBytes(snapshot.hardware.memoryBytes)}</dd></div><div><dt>Driver</dt><dd>{snapshot.hardware.driver ?? "Not reported"}</dd></div></dl>
            </>
          ) : <p className="muted">{bootstrapError ?? "Reading local hardware…"}</p>}
        </section>
        {selected ? (
          <section className="selection-card">
            <p className="eyebrow">Selected artifact</p>
            <strong>{selected.id}</strong>
            {selected.artifacts.length > 1 ? <select className="artifact-select" aria-label="Model artifact" value={selectedArtifact?.filename ?? ""} onChange={(event) => void chooseArtifact(event.target.value)}>{selected.artifacts.map((artifact) => <option key={artifact.filename} value={artifact.filename}>{artifact.filename} · {formatBytes(artifact.sizeBytes)}</option>)}</select> : <span>{selectedArtifact?.filename ?? "No GGUF artifact"}</span>}
            {assessment ? <div className={`fit-badge ${assessment.status === "unlikely-to-fit" ? "warning" : ""}`}>{assessment.status === "unlikely-to-fit" ? <WarningCircle size={14} /> : <Check size={14} />}{fitLabel(assessment)}</div> : <div className="fit-badge neutral">Size metadata required</div>}
            {assessment && <p className="evidence">{assessment.evidenceLabel}</p>}
            <button className="primary-button" type="button" onClick={saveDefault} disabled={!selectedArtifact}>{defaultSaved ? <><Check size={16} />Default for new tasks</> : "Set as default"}</button>
            <button className="secondary-button" type="button" onClick={downloadState.state === "running" ? cancelSelectedDownload : downloadSelected}>{downloadState.state === "running" ? <><CircleNotch className="spin" size={16} />Cancel download</> : <><DownloadSimple size={16} />Download model</>}</button>
            {downloadState.message && <p className={downloadState.state === "error" ? "download-note error" : "download-note"}>{downloadState.message}</p>}
          </section>
        ) : (
          <section className="inspector-empty"><TerminalWindow size={20} /><p>Select a model to inspect its fit and runtime settings.</p></section>
        )}
        <div className="inspector-spacer" />
        <section className="runtime-card"><span className={`status-dot ${snapshot?.runtime.state === "unconfigured" || snapshot?.runtime.state === "unavailable" ? "warning" : ""}`} /><div><strong>{snapshot ? `Control plane ${snapshot.runtime.state}` : "Inspecting control plane"}</strong><small>{snapshot?.runtime.profile ?? "Waiting for profile"}</small></div></section>
        <p className="hardware-line">{hardwareLine}</p>
      </aside>
    </div>
  );
}

function NavButton({ active, label, onClick, children }: { active: boolean; label: string; onClick: () => void; children: React.ReactNode }) {
  return <button type="button" className={active ? "active" : ""} aria-current={active ? "page" : undefined} onClick={onClick}>{children}<span>{label}</span></button>;
}

function TaskView({ snapshot, bootstrapError, onExplore, taskRun, onRun, onCancel }: { snapshot: BootstrapSnapshot | null; bootstrapError: string | null; onExplore: () => void; taskRun: TaskRun | null; onRun: (prompt: string) => Promise<void>; onCancel: () => void }) {
  const [draft, setDraft] = useState("");
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const prompt = draft.trim();
    if (!prompt || taskRun?.state === "running") return;
    setDraft("");
    void onRun(prompt);
  };
  const running = taskRun?.state === "running" || taskRun?.state === "cancelling";
  return <div className="task-view"><div className={`task-stream ${taskRun ? "has-run" : ""}`}>{taskRun ? <div className="transcript"><article className="user-message"><span>You</span><p>{taskRun.prompt}</p></article><article className="assistant-message"><span><Sparkle size={13} weight="fill" />Alpine · Pi</span>{taskRun.state === "error" ? <div className="error-banner">{taskRun.error}</div> : <p>{taskRun.response || (taskRun.state === "cancelled" ? "Task cancelled." : taskRun.state === "cancelling" ? "Cancelling the local task…" : "Starting the local harness…")}</p>}</article></div> : <><div className="task-kicker"><span className="status-dot" />Local workspace ready</div><h1>What should we run locally?</h1><p>Alpine understands this machine, finds compatible GGUF models, and turns experiments into reusable runtime profiles.</p><div className="system-summary"><Cpu size={19} /><div><strong>{snapshot?.hardware.cpu ?? (bootstrapError ? "Hardware scan unavailable" : "Inspecting your machine")}</strong><span>{snapshot ? `${formatBytes(snapshot.hardware.memoryBytes)} memory · ${snapshot.hardware.gpu ?? "CPU-only"}` : bootstrapError ?? "Collecting CPU, GPU, memory, drivers, and runtime availability."}</span></div></div><button className="secondary-button explore" type="button" onClick={onExplore}>Explore models<ArrowDown size={15} /></button></>}</div><form className="composer" onSubmit={submit}><textarea aria-label="Task prompt" value={draft} onChange={(event) => setDraft(event.target.value)} placeholder="Ask Alpine to find, download, or analyze a local model…" /><div className="composer-footer"><button type="button"><Plus size={16} />Add context</button><div><button type="button">Pi <CaretDown size={13} /></button><button type="button">Default model <CaretDown size={13} /></button><button className="send" type={running ? "button" : "submit"} aria-label={running ? "Cancel task" : "Run task"} disabled={taskRun?.state === "cancelling" || (!running && !draft.trim())} onClick={running ? onCancel : undefined}>{running ? <StopCircle size={17} weight="fill" /> : <ArrowDown size={17} weight="bold" />}</button></div></div></form></div>;
}

function ModelsView({ query, setQuery, search, state, error, models, selected, chooseModel }: { query: string; setQuery: (value: string) => void; search: (event: FormEvent) => void; state: "idle" | "loading" | "error"; error: string | null; models: ModelSearchResult[]; selected: ModelSearchResult | null; chooseModel: (model: ModelSearchResult) => void }) {
  return <div className="content-view"><div className="page-heading"><p className="eyebrow">Hugging Face registry</p><h1>Find a model for this machine</h1><p>Search GGUF repositories, inspect exact artifacts, then estimate fit before committing disk space.</p></div><form className="model-search" onSubmit={search}><MagnifyingGlass size={18} /><input type="search" aria-label="Search Hugging Face" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search models, publishers, or architectures" /><button type="submit" disabled={state === "loading"}>{state === "loading" ? <CircleNotch className="spin" size={17} /> : "Search"}</button></form>{error && <div className="error-banner">{error}</div>}<div className="result-header"><span>{models.length ? `${models.length} GGUF repositories` : "Search the Hub to begin"}</span><button type="button">Most downloaded <CaretDown size={13} /></button></div><div className="model-list">{models.map((model) => <button type="button" key={model.id} className={`model-row ${selected?.id === model.id ? "selected" : ""}`} onClick={() => chooseModel(model)}><div className="model-avatar">{model.publisher.slice(0, 1).toUpperCase()}</div><div className="model-identity"><strong>{model.id}</strong><span>{model.artifacts[0]?.filename ?? "GGUF artifact metadata pending"}</span></div><div className="model-stats"><span>{formatCount(model.downloads)} downloads</span><span>{formatCount(model.likes)} likes</span></div><div className="model-size">{formatBytes(model.artifacts[0]?.sizeBytes ?? 0)}</div></button>)}</div>{state === "idle" && models.length === 0 && <div className="empty-search"><HardDrives size={26} /><strong>No results loaded</strong><span>Try “Qwen”, “Llama”, or a Hugging Face repository name.</span></div>}</div>;
}

function BrowserView({ address, setAddress }: { address: string; setAddress: (value: string) => void }) {
  const [openAddress, setOpenAddress] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const open = () => {
    try {
      const parsed = new URL(address);
      if (!(["http:", "https:"].includes(parsed.protocol) && ["localhost", "127.0.0.1", "[::1]"].includes(parsed.hostname))) {
        throw new Error("The first browser surface is limited to local preview addresses.");
      }
      setError(null);
      setOpenAddress(parsed.href);
    } catch (nextError) {
      setOpenAddress(null);
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    }
  };
  const isCurrentShell = openAddress ? new URL(openAddress).origin === window.location.origin : false;
  return <div className="browser-view"><div className="browser-toolbar"><div className="traffic-lights"><i /><i /><i /></div><input aria-label="Browser address" value={address} onChange={(event) => setAddress(event.target.value)} /><button type="button" onClick={open}>Open</button></div>{error ? <div className="browser-canvas"><Browser size={35} /><h2>Preview blocked</h2><p>{error}</p></div> : openAddress && !isCurrentShell ? <iframe className="browser-frame" title="Browser preview" src={openAddress} /> : <div className="browser-canvas"><Browser size={35} /><h2>Browser artifact surface</h2><p>{isCurrentShell ? "That address is already the Alpine shell. Enter another local development URL to inspect it here." : "Open localhost previews and generated artifacts beside the active task. External authenticated browsing remains a separate authority."}</p><code>{address}</code></div>}</div>;
}

function DownloadsView({ downloads, error }: { downloads: DownloadedModel[]; error: string | null }) {
  return <div className="content-view"><div className="page-heading"><p className="eyebrow">Local registry</p><h1>Downloads</h1><p>Completed GGUF artifacts and resumable partial transfers in the configured Alpine installation.</p></div>{error && <div className="error-banner">{error}</div>}<div className="result-header"><span>{downloads.length ? `${downloads.length} local artifacts` : "No local artifacts"}</span><span>Private to this machine</span></div><div className="download-list">{downloads.map((download) => <div className="download-row" key={download.filename}><div className="download-icon"><DownloadSimple size={17} /></div><div><strong>{download.filename}</strong><span>{download.state === "partial" ? "Partial transfer · resume by downloading the same artifact" : "Installed GGUF artifact"}</span></div><span className={`download-state ${download.state}`}>{download.state}</span><b>{formatBytes(download.sizeBytes)}</b></div>)}</div>{downloads.length === 0 && <div className="empty-search"><DownloadSimple size={26} /><strong>No downloads yet</strong><span>Find a Hugging Face model, choose an exact GGUF artifact, and start its verified transfer.</span></div>}</div>;
}

function AnalysisView({ snapshot, state, onRun }: { snapshot: BootstrapSnapshot | null; state: { state: "idle" | "running" | "done" | "error"; report?: RuntimeProbeReport; error?: string }; onRun: () => Promise<void> }) {
  const selected = snapshot?.settings.defaultModel?.filename;
  const configured = snapshot?.runtime.model;
  const modelMatches = Boolean(selected && configured && selected.toLowerCase() === configured.toLowerCase());
  const runtimeReady = snapshot?.runtime.state === "configured" || snapshot?.runtime.state === "running";
  const canRun = modelMatches && runtimeReady;
  return <div className="content-view analysis-view"><div className="page-heading"><p className="eyebrow">Evidence lab</p><h1>Analysis</h1><p>Start with one bounded local diagnostic. Alpine labels measurements separately from qualification and never promotes a profile from this screen.</p></div><section className="analysis-summary"><div><span>Default artifact</span><strong>{selected ?? "Not selected"}</strong></div><div><span>Runtime model</span><strong>{configured ?? "Not configured"}</strong></div><div><span>Profile</span><strong>{snapshot?.runtime.profile ?? "Loading…"}</strong></div></section><div className="analysis-steps"><div className={modelMatches ? "complete" : "blocked"}><span>01</span><div><strong>Identity match</strong><p>{modelMatches ? "The selected default matches the configured llama.cpp model." : "Select the active runtime model in Settings, or configure the downloaded artifact in Alpine first."}</p></div></div><div className={runtimeReady ? "complete" : "blocked"}><span>02</span><div><strong>Runtime preflight</strong><p>{snapshot?.runtime.detail ?? "Inspecting the local control plane…"}</p></div></div><div className={state.report ? "complete" : "pending"}><span>03</span><div><strong>Measured diagnostic</strong><p>{state.report ? `${state.report.latencyMs} ms end-to-end · ${state.report.outputTokens ?? "unknown"} output tokens · ${state.report.qualityPass ? "exact-output pass" : "exact-output miss"}` : "A fixed local prompt checks end-to-end llama.cpp health without claiming qualification."}</p></div></div></div>{state.error && <div className="error-banner">{state.error}</div>}{state.report && <div className="evidence-banner"><Check size={15} /><div><strong>{state.report.evidenceLabel}</strong><span>{state.report.model} · {state.report.profile}</span></div></div>}<button className="primary-button analysis-run" type="button" disabled={!canRun || state.state === "running"} onClick={() => void onRun()}>{state.state === "running" ? <><CircleNotch className="spin" size={16} />Running local diagnostic…</> : <><Gauge size={16} />Run measured diagnostic</>}</button>{!canRun && <p className="analysis-help">The button stays disabled until the selected default and configured runtime refer to the same exact model.</p>}</div>;
}

function SettingsView({ snapshot, onSave, onUseActive }: { snapshot: BootstrapSnapshot | null; onSave: (update: SettingsUpdate) => Promise<void>; onUseActive: () => Promise<void> }) {
  const [installRoot, setInstallRoot] = useState("");
  const [profile, setProfile] = useState("stable-16k");
  const [localMetrics, setLocalMetrics] = useState(true);
  const [saveState, setSaveState] = useState<{ state: "idle" | "saving" | "done" | "error"; message?: string }>({ state: "idle" });
  useEffect(() => {
    if (!snapshot) return;
    setInstallRoot(snapshot.settings.installRoot);
    setProfile(snapshot.settings.defaultProfile);
    setLocalMetrics(snapshot.settings.localMetricsEnabled);
  }, [snapshot]);
  const save = async (event: FormEvent) => {
    event.preventDefault();
    setSaveState({ state: "saving" });
    try {
      await onSave({ installRoot, defaultProfile: profile, localMetricsEnabled: localMetrics });
      setSaveState({ state: "done", message: "Settings saved and runtime rechecked." });
    } catch (error) {
      setSaveState({ state: "error", message: error instanceof Error ? error.message : String(error) });
    }
  };
  const profiles = Array.from(new Set([profile, ...(snapshot?.runtime.availableProfiles ?? [])]));
  return <form className="content-view settings-view" onSubmit={save}><div className="page-heading"><p className="eyebrow">Workspace preferences</p><h1>Settings</h1><p>Configure the Alpine installation, default model and profile, privacy, and local diagnostics.</p></div><section className="settings-group"><h2>Runtime</h2><SettingRow title="Default harness" copy="Used when a new local task starts."><button type="button" disabled>Pi SDK · experimental</button></SettingRow><SettingRow title="Default model" copy="New tasks inherit this exact artifact."><div className="setting-action"><code>{snapshot?.settings.defaultModel?.filename ?? "Not selected"}</code>{snapshot?.runtime.model && <button type="button" onClick={() => void onUseActive()}>Use active model</button>}</div></SettingRow><SettingRow title="Default profile" copy="The Alpine profile started before Pi connects."><select aria-label="Default profile" value={profile} onChange={(event) => setProfile(event.target.value)}>{profiles.map((value) => <option key={value} value={value}>{value}</option>)}</select></SettingRow><SettingRow title="Runtime status" copy={snapshot?.runtime.detail ?? "Inspecting the local control plane…"}><span className={`settings-status ${snapshot?.runtime.state ?? "loading"}`}>{snapshot?.runtime.state ?? "loading"}</span></SettingRow></section><section className="settings-group"><h2>Storage</h2><SettingRow title="Alpine installation root" copy="Models are downloaded into its models directory; runtime configuration stays authoritative."><input aria-label="Alpine installation root" value={installRoot} onChange={(event) => setInstallRoot(event.target.value)} /></SettingRow></section><section className="settings-group"><h2>Privacy & diagnostics</h2><SettingRow title="Local performance measurements" copy="Records browser timings only. Prompts, credentials, and repository content are excluded."><button className={`toggle ${localMetrics ? "on" : ""}`} type="button" role="switch" aria-checked={localMetrics} onClick={() => setLocalMetrics((value) => !value)}><span /></button></SettingRow></section>{saveState.message && <p className={saveState.state === "error" ? "settings-message error" : "settings-message"}>{saveState.message}</p>}<button className="primary-button settings-save" type="submit" disabled={!snapshot || saveState.state === "saving"}>{saveState.state === "saving" ? "Saving…" : "Save settings"}</button></form>;
}

function SettingRow({ title, copy, children }: { title: string; copy: string; children: React.ReactNode }) { return <div className="setting-row"><div><strong>{title}</strong><p>{copy}</p></div><div>{children}</div></div>; }
function titleFor(view: View) { return ({ task: "New task", models: "Models", downloads: "Downloads", analysis: "Analysis", browser: "Browser", settings: "Settings" } as const)[view]; }
