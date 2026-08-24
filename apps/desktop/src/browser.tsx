import {
  ArrowClockwise,
  ArrowLeft,
  ArrowRight,
  Browser,
  CircleNotch,
  Plus,
  ShieldCheck,
  X,
} from "@phosphor-icons/react";
import { FormEvent, useEffect, useRef, useState } from "react";
import type {
  BrowserAdapter,
  BrowserBounds,
  BrowserEvent,
} from "./desktop";

type BrowserTab = {
  id: string;
  title: string;
  url: string;
  loading: boolean;
};

type AccessRequest = {
  tabId: string;
  url: string;
  host: string;
};

const initialTab: BrowserTab = {
  id: "browser-1",
  title: "New tab",
  url: "about:blank",
  loading: false,
};

export function BrowserSurface({ browser }: { browser: BrowserAdapter }) {
  const [tabs, setTabs] = useState<BrowserTab[]>([initialTab]);
  const [activeId, setActiveId] = useState(initialTab.id);
  const [address, setAddress] = useState("");
  const [accessRequest, setAccessRequest] = useState<AccessRequest | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [downloadNote, setDownloadNote] = useState<string | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const activeIdRef = useRef(activeId);
  const tabSequence = useRef(1);

  activeIdRef.current = activeId;
  const activeTab = tabs.find((tab) => tab.id === activeId) ?? tabs[0];

  const freshTab = (): BrowserTab => {
    tabSequence.current += 1;
    return {
      id: `browser-${tabSequence.current}`,
      title: "New tab",
      url: "about:blank",
      loading: false,
    };
  };

  const bounds = (): BrowserBounds => {
    const rect = viewportRef.current?.getBoundingClientRect();
    return {
      x: Math.max(0, Math.round(rect?.x ?? 0)),
      y: Math.max(0, Math.round(rect?.y ?? 0)),
      width: Math.max(1, Math.round(rect?.width || 520)),
      height: Math.max(1, Math.round(rect?.height || 480)),
    };
  };

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    browser.subscribe((event) => {
      if (event.kind === "newTabRequested") {
        const tab = freshTab();
        setTabs((current) => [...current, tab]);
        setActiveId(tab.id);
        activeIdRef.current = tab.id;
        setAddress(event.url);
        setAccessRequest(null);
        void navigateTab(tab.id, event.url, false);
        return;
      }
      if (event.kind === "accessRequested") {
        setActiveId(event.tabId);
        activeIdRef.current = event.tabId;
        setAddress(event.url);
        setAccessRequest({ tabId: event.tabId, url: event.url, host: event.host });
        return;
      }
      applyBrowserEvent(
        event,
        setTabs,
        setAddress,
        setDownloadNote,
        activeIdRef.current,
      );
    }).then((dispose) => { unlisten = dispose; }).catch((cause: unknown) => setError(errorMessage(cause)));
    return () => unlisten?.();
  }, [browser]);

  useEffect(() => {
    let frame: number | null = null;
    const sync = () => {
      if (frame != null) window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        frame = null;
        void browser.setActive({ tabId: activeId, bounds: bounds() }).catch((cause: unknown) => setError(errorMessage(cause)));
      });
    };
    sync();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(sync);
    if (viewportRef.current) observer?.observe(viewportRef.current);
    window.addEventListener("resize", sync);
    return () => {
      if (frame != null) window.cancelAnimationFrame(frame);
      observer?.disconnect();
      window.removeEventListener("resize", sync);
      void browser.setActive(null);
    };
  }, [activeId, browser]);

  const navigateTab = async (tabId: string, nextAddress: string, allowHost: boolean) => {
    const trimmed = nextAddress.trim();
    if (!trimmed) return;
    setError(null);
    try {
      const result = await browser.navigate({
        tabId,
        address: trimmed,
        allowHost,
        bounds: bounds(),
      });
      if (result.status === "approval-required" && result.host) {
        setAccessRequest({ tabId, url: result.url, host: result.host });
        return;
      }
      setAccessRequest(null);
      if (activeIdRef.current === tabId) setAddress(result.url);
      setTabs((current) => current.map((tab) => tab.id === tabId
        ? { ...tab, url: result.url, loading: true }
        : tab));
      await browser.setActive({ tabId, bounds: bounds() });
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    void navigateTab(activeId, address, false);
  };

  const addTab = () => {
    const tab = freshTab();
    setTabs((current) => [...current, tab]);
    setActiveId(tab.id);
    activeIdRef.current = tab.id;
    setAddress("");
    setAccessRequest(null);
  };

  const runCommand = async (command: "back" | "forward" | "reload") => {
    setError(null);
    try {
      await browser.command(activeId, command);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  const closeTab = async (tabId: string) => {
    setError(null);
    try {
      await browser.command(tabId, "close");
    } catch (cause) {
      setError(errorMessage(cause));
      return;
    }
    if (tabs.length === 1) {
      const replacement = freshTab();
      setTabs([replacement]);
      setActiveId(replacement.id);
      activeIdRef.current = replacement.id;
      setAddress("");
      setAccessRequest(null);
      return;
    }
    setTabs((current) => current.filter((tab) => tab.id !== tabId));
    if (tabId === activeId) {
      const remaining = tabs.filter((tab) => tab.id !== tabId);
      const next = remaining.at(-1)!;
      setActiveId(next.id);
      activeIdRef.current = next.id;
      setAddress(next.url === "about:blank" ? "" : next.url);
    }
  };

  return <section className="browser-surface" aria-label="Shared browser">
    <div className="browser-tabs" role="tablist" aria-label="Browser tabs">
      {tabs.map((tab) => <div className={`browser-tab ${tab.id === activeId ? "active" : ""}`} role="presentation" key={tab.id}>
        <button
          type="button"
          role="tab"
          aria-selected={tab.id === activeId}
          onClick={() => {
            setActiveId(tab.id);
            activeIdRef.current = tab.id;
            setAddress(tab.url === "about:blank" ? "" : tab.url);
            setAccessRequest(null);
          }}
        >{tab.loading ? <CircleNotch className="spin" size={13} /> : <Browser size={13} />}<span>{tab.title}</span></button>
        <button type="button" aria-label={`Close ${tab.title}`} onClick={() => void closeTab(tab.id)}><X size={12} /></button>
      </div>)}
      <button className="browser-new-tab" type="button" aria-label="New browser tab" onClick={addTab}><Plus size={14} /></button>
    </div>
    <div className="browser-toolbar">
      <button type="button" aria-label="Back" disabled={activeTab.url === "about:blank"} onClick={() => void runCommand("back")}><ArrowLeft size={15} /></button>
      <button type="button" aria-label="Forward" disabled={activeTab.url === "about:blank"} onClick={() => void runCommand("forward")}><ArrowRight size={15} /></button>
      <button type="button" aria-label="Reload" disabled={activeTab.url === "about:blank"} onClick={() => void runCommand("reload")}><ArrowClockwise size={15} /></button>
      <form onSubmit={submit}>
        <ShieldCheck size={14} />
        <input aria-label="Browser address" value={address} onChange={(event) => setAddress(event.target.value)} placeholder="Search or enter an address" spellCheck={false} />
        <button type="submit">Go</button>
      </form>
    </div>
    {accessRequest && <div className="browser-permission" role="alert">
      <div><ShieldCheck size={17} /><span><strong>Allow {accessRequest.host}?</strong><small>Alpine asks before opening a new website.</small></span></div>
      <div><button type="button" onClick={() => setAccessRequest(null)}>Cancel</button><button className="primary-button" type="button" onClick={() => void navigateTab(accessRequest.tabId, accessRequest.url, true)}>Allow once</button></div>
    </div>}
    {error && <div className="error-banner">{error}</div>}
    {downloadNote && <p className="browser-download-note" aria-live="polite">{downloadNote}</p>}
    <div className="browser-viewport" ref={viewportRef}>
      {!browser.nativeSurface && activeTab.url !== "about:blank" && <iframe title="Browser page" src={activeTab.url} sandbox="allow-forms allow-modals allow-pointer-lock allow-popups allow-same-origin allow-scripts" />}
      {activeTab.url === "about:blank" && <div className="browser-empty"><Browser size={22} /><p>Open a local app or website without leaving the task.</p><small>New websites ask for access first.</small></div>}
    </div>
  </section>;
}

function applyBrowserEvent(
  event: BrowserEvent,
  setTabs: React.Dispatch<React.SetStateAction<BrowserTab[]>>,
  setAddress: React.Dispatch<React.SetStateAction<string>>,
  setDownloadNote: React.Dispatch<React.SetStateAction<string | null>>,
  activeId: string,
) {
  if (event.kind === "page") {
    setTabs((current) => current.map((tab) => tab.id === event.tabId
      ? { ...tab, url: event.url, loading: event.loading }
      : tab));
    if (event.tabId === activeId) setAddress(event.url);
    return;
  }
  if (event.kind === "title") {
    setTabs((current) => current.map((tab) => tab.id === event.tabId
      ? { ...tab, title: event.title || "New tab" }
      : tab));
    return;
  }
  if (event.kind === "accessRequested") return;
  if (event.kind === "newTabRequested") return;
  const filename = event.path?.split(/[\\/]/).at(-1) ?? "download";
  setDownloadNote(event.state === "started"
    ? `Downloading ${filename}…`
    : event.state === "completed"
      ? `Saved ${filename}`
      : `${filename} could not be downloaded`);
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
