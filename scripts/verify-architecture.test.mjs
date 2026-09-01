import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { verifyArchitecture } from "./verify-architecture.mjs";

async function fixture(overrides = {}) {
  const root = await mkdtemp(join(tmpdir(), "alpine-architecture-"));
  const files = {
    "Cargo.toml": "[package]\nname='root'\nversion='0.1.0'\n",
    "Cargo.lock": "",
    "rust-toolchain.toml": "[toolchain]\nchannel='1.85.0'\n",
    "pyproject.toml": "[project]\nname='compat'\nversion='0.1.0'\n",
    "apps/desktop/package.json": "{\"name\":\"desktop\"}",
    "apps/desktop/package-lock.json": "{}",
    "apps/desktop/src-tauri/Cargo.toml": "[package]\nname='desktop'\nversion='0.1.0'\n",
    "apps/desktop/src-tauri/Cargo.lock": "",
    "apps/desktop/src-tauri/rust-toolchain.toml": "[toolchain]\nchannel='1.88.0'\n",
    "apps/desktop/src/harness/pi.ts": 'import { Agent } from "@earendil-works/pi-agent-core";\n',
    "apps/desktop/src/task-execution.ts": 'import type { AgentEvent } from "@earendil-works/pi-agent-core";\nclient.appendTaskEvent();\n',
    "apps/desktop/src/desktop.ts": "export interface Client { appendTaskEvent(): void }\n",
    ".github/workflows/verify.yml": "name: verify\njobs:\n  root:\n    name: control-plane\n    steps:\n      - run: root-check\n  desktop:\n    name: desktop\n    steps:\n      - run: desktop-check\n",
  };
  Object.assign(files, overrides.files ?? {});
  for (const [path, content] of Object.entries(files)) {
    const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
    if (parent) await mkdir(join(root, parent), { recursive: true });
    await writeFile(join(root, path), content);
  }
  const policy = {
    schema: 1,
    manifestDiscovery: {
      names: ["Cargo.toml", "package.json", "pyproject.toml"],
      ignoredDirectories: ["node_modules", "target", "dist", ".git"],
    },
    packages: [
      { kind: "cargo", manifest: "Cargo.toml", lockfile: "Cargo.lock", toolchain: "rust-toolchain.toml", verificationJob: "control-plane" },
      { kind: "python", manifest: "pyproject.toml", verificationJob: "control-plane" },
      { kind: "npm", manifest: "apps/desktop/package.json", lockfile: "apps/desktop/package-lock.json", verificationJob: "desktop" },
      { kind: "cargo", manifest: "apps/desktop/src-tauri/Cargo.toml", lockfile: "apps/desktop/src-tauri/Cargo.lock", toolchain: "apps/desktop/src-tauri/rust-toolchain.toml", verificationJob: "desktop" },
    ],
    providerImports: {
      packages: ["@earendil-works/pi-agent-core", "@earendil-works/pi-ai"],
      roots: ["apps/desktop/src", "apps/desktop/scripts"],
      ignoredDirectories: ["node_modules", "dist"],
      allowed: [{ path: "apps/desktop/src/harness/pi.ts", reason: "adapter" }],
      temporaryExceptions: [{ path: "apps/desktop/src/task-execution.ts", issue: 49, reason: "legacy provider event leak" }],
    },
    rendererMutationBoundary: {
      roots: ["apps/desktop/src"],
      ignoredDirectories: ["node_modules", "dist"],
      symbols: ["appendTaskEvent"],
      temporaryAllowedFiles: [
        { path: "apps/desktop/src/desktop.ts", issue: 47, reason: "legacy declaration" },
        { path: "apps/desktop/src/task-execution.ts", issue: 47, reason: "legacy orchestrator" },
      ],
    },
    workflowContract: {
      path: ".github/workflows/verify.yml",
      requiredCheckNames: ["control-plane", "desktop"],
      requiredSnippets: ["root-check", "desktop-check"],
    },
    generatedContracts: [],
    manualVerification: [{ id: "native-app", owner: "maintainer", issue: 43 }],
    ...overrides.policy,
  };
  await mkdir(join(root, "config"), { recursive: true });
  await writeFile(join(root, "config/architecture-policy.json"), JSON.stringify(policy));
  return root;
}

test("accepts the declared package and temporary architecture boundaries", async () => {
  const root = await fixture();
  const summary = await verifyArchitecture({ root });
  assert.equal(summary.packages, 4);
  assert.equal(summary.notes.length, 3);
});

test("rejects an unlisted package manifest", async () => {
  const root = await fixture({ files: { "tools/new/package.json": "{}" } });
  await assert.rejects(() => verifyArchitecture({ root }), /unlisted package manifest: tools\/new\/package\.json/);
});

test("rejects a provider import outside the adapter boundary", async () => {
  const root = await fixture({ files: { "apps/desktop/src/rogue.ts": 'import { Agent } from "@earendil-works/pi-agent-core";\n' } });
  await assert.rejects(() => verifyArchitecture({ root }), /provider package import outside the declared adapter boundary: apps\/desktop\/src\/rogue\.ts/);
});

test("rejects a new renderer durable-state mutation caller", async () => {
  const root = await fixture({ files: { "apps/desktop/src/rogue.ts": "client.appendTaskEvent();\n" } });
  await assert.rejects(() => verifyArchitecture({ root }), /renderer durable-state primitive used outside the temporary boundary: apps\/desktop\/src\/rogue\.ts/);
});

test("rejects a workflow that omits a required desktop command", async () => {
  const root = await fixture({
    files: {
      ".github/workflows/verify.yml": "name: verify\njobs:\n  root:\n    name: control-plane\n    steps:\n      - run: root-check\n  desktop:\n    name: desktop\n",
    },
  });
  await assert.rejects(
    () => verifyArchitecture({ root }),
    /verification workflow is missing required command: desktop-check/,
  );
});
