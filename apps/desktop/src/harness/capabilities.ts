export type RuntimeCapabilityStatus = "available" | "limited" | "unavailable";

export type RuntimeCapability = {
  id: string;
  label: string;
  status: RuntimeCapabilityStatus;
  detail: string;
};

export const PI_RUNTIME_CAPABILITIES: RuntimeCapability[] = [
  { id: "prompt-stream", label: "Prompt and streaming", status: "available", detail: "Streams text from the selected local model into a durable Task." },
  { id: "cancel", label: "Cancel", status: "available", detail: "Stops the active Pi request and settles the Task as cancelled." },
  { id: "steer", label: "Steer current run", status: "available", detail: "Queues one direction for the active request." },
  { id: "follow-up", label: "Queue follow-up", status: "available", detail: "Queues one prompt to run after the current request." },
  { id: "history-restore", label: "Restore Task history", status: "available", detail: "Reconstructs Pi messages from Alpine-owned Task Messages." },
  { id: "project-tools", label: "Project tools", status: "available", detail: "List, read, and search project files; propose exact edits and commands." },
  { id: "tool-approval", label: "Tool approval", status: "available", detail: "Edits and commands wait for an exact, durable operator decision." },
  { id: "images", label: "Image prompts", status: "unavailable", detail: "The first local model contract is text-only." },
  { id: "skills", label: "Skills and templates", status: "unavailable", detail: "No verified Alpine capability registry is installed yet." },
  { id: "graph-context", label: "Graph context", status: "unavailable", detail: "Graphify was assessed, but Alpine will not install hooks or execute an unpinned ambient CLI inside a project. A managed, opt-in adapter remains a release gate." },
  { id: "compaction", label: "Harness compaction", status: "unavailable", detail: "Pi 0.84.2 exposes this in its experimental harness surface, but the shipped implementation is not complete." },
  { id: "session-tree", label: "Session tree navigation", status: "unavailable", detail: "Alpine keeps linear durable Tasks; Pi tree navigation is not wired." },
  { id: "lanes", label: "Parallel lanes", status: "unavailable", detail: "Pi lanes are not exposed by the embedded low-level Agent adapter." },
];
