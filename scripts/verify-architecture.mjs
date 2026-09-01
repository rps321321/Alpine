import { readFile, readdir, stat } from "node:fs/promises";
import { dirname, extname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_PATH = fileURLToPath(import.meta.url);
const DEFAULT_ROOT = resolve(dirname(SCRIPT_PATH), "..");
const DEFAULT_POLICY = "config/architecture-policy.json";

const normalize = (value) => value.split(sep).join("/");

async function readJson(path) {
  const text = await readFile(path, "utf8");
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`invalid JSON at ${path}: ${error.message}`);
  }
}

async function exists(path) {
  try {
    await stat(path);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

async function walkFiles(root, ignoredDirectories = []) {
  const ignored = new Set(ignoredDirectories);
  const output = [];
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      if (entry.isDirectory() && ignored.has(entry.name)) continue;
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await visit(path);
      else if (entry.isFile()) output.push(path);
    }
  }
  await visit(root);
  return output;
}

function importSpecifiers(source) {
  const patterns = [
    /\bfrom\s+["']([^"']+)["']/g,
    /\bimport\s*["']([^"']+)["']/g,
    /\bimport\s*\(\s*["']([^"']+)["']\s*\)/g,
    /\brequire\s*\(\s*["']([^"']+)["']\s*\)/g,
  ];
  const found = new Set();
  for (const pattern of patterns) {
    for (const match of source.matchAll(pattern)) found.add(match[1]);
  }
  return [...found];
}

function matchesPackage(specifier, packageName) {
  return specifier === packageName || specifier.startsWith(`${packageName}/`);
}

function validatePolicyShape(policy) {
  const errors = [];
  if (policy?.schema !== 1) errors.push("architecture policy schema must be 1");
  if (!Array.isArray(policy?.packages) || !policy.packages.length) {
    errors.push("architecture policy must declare at least one package");
  }
  if (!Array.isArray(policy?.manifestDiscovery?.names) || !policy.manifestDiscovery.names.length) {
    errors.push("manifestDiscovery.names must be a non-empty array");
  }
  for (const rule of policy?.manualVerification ?? []) {
    if (!rule.id || !rule.owner || !Number.isInteger(rule.issue) || rule.issue < 1) {
      errors.push("manual verification rules require id, owner, and positive issue");
    }
  }
  return errors;
}

async function verifyManifests(root, policy, errors) {
  const discovery = policy.manifestDiscovery;
  const names = new Set(discovery.names);
  const files = await walkFiles(root, discovery.ignoredDirectories ?? []);
  const discovered = new Set(
    files
      .filter((path) => names.has(path.split(sep).at(-1)))
      .map((path) => normalize(relative(root, path))),
  );
  const declared = new Set(policy.packages.map((entry) => entry.manifest));

  for (const path of [...discovered].sort()) {
    if (!declared.has(path)) errors.push(`unlisted package manifest: ${path}`);
  }
  for (const packageEntry of policy.packages) {
    const { manifest, lockfile, toolchain, verificationJob, kind } = packageEntry;
    if (!manifest || !kind || !verificationJob) {
      errors.push(`invalid package declaration: ${JSON.stringify(packageEntry)}`);
      continue;
    }
    if (!discovered.has(manifest)) errors.push(`declared package manifest is missing: ${manifest}`);
    for (const supportPath of [lockfile, toolchain].filter(Boolean)) {
      if (!(await exists(join(root, supportPath)))) {
        errors.push(`declared package support file is missing: ${supportPath}`);
      }
    }
  }
}

async function sourceFiles(root, roots, ignoredDirectories = []) {
  const extensions = new Set([".ts", ".tsx", ".js", ".mjs", ".cjs"]);
  const output = [];
  for (const sourceRoot of roots) {
    const absolute = join(root, sourceRoot);
    if (!(await exists(absolute))) continue;
    for (const path of await walkFiles(absolute, ignoredDirectories)) {
      if (extensions.has(extname(path))) output.push(path);
    }
  }
  return output;
}

async function verifyProviderImports(root, policy, errors, notes) {
  const rule = policy.providerImports;
  if (!rule) return;
  const allowed = new Set((rule.allowed ?? []).map((entry) => entry.path));
  const exceptions = new Map(
    (rule.temporaryExceptions ?? []).map((entry) => [entry.path, entry]),
  );
  const observed = new Map();

  for (const path of await sourceFiles(root, rule.roots, rule.ignoredDirectories ?? [])) {
    const relativePath = normalize(relative(root, path));
    const source = await readFile(path, "utf8");
    const providerImports = importSpecifiers(source).filter((specifier) =>
      rule.packages.some((packageName) => matchesPackage(specifier, packageName)),
    );
    if (!providerImports.length) continue;
    observed.set(relativePath, providerImports);
    if (!allowed.has(relativePath) && !exceptions.has(relativePath)) {
      errors.push(
        `provider package import outside the declared adapter boundary: ${relativePath} (${providerImports.join(", ")})`,
      );
    }
  }

  for (const entry of rule.allowed ?? []) {
    if (!observed.has(entry.path)) {
      errors.push(`stale provider-import allowlist entry: ${entry.path}`);
    }
  }
  for (const entry of rule.temporaryExceptions ?? []) {
    if (!Number.isInteger(entry.issue) || entry.issue < 1 || !entry.reason) {
      errors.push(`invalid provider-import exception: ${entry.path}`);
    } else if (!observed.has(entry.path)) {
      errors.push(`resolved provider-import exception must be removed: ${entry.path}`);
    } else {
      notes.push(`temporary provider-import exception: ${entry.path} (issue #${entry.issue})`);
    }
  }
}

function isTestPath(path) {
  return /(?:^|\/)(?:tests?|__tests__)(?:\/|$)/.test(path) || /\.(?:test|spec)\.[cm]?[jt]sx?$/.test(path);
}

async function verifyMutationBoundary(root, policy, errors, notes) {
  const rule = policy.rendererMutationBoundary;
  if (!rule) return;
  const allowed = new Map((rule.temporaryAllowedFiles ?? []).map((entry) => [entry.path, entry]));
  const observed = new Set();
  for (const path of await sourceFiles(root, rule.roots, rule.ignoredDirectories ?? [])) {
    const relativePath = normalize(relative(root, path));
    if (isTestPath(relativePath)) continue;
    const source = await readFile(path, "utf8");
    const used = rule.symbols.filter((symbol) => new RegExp(`\\b${symbol}\\b`).test(source));
    if (!used.length) continue;
    observed.add(relativePath);
    if (!allowed.has(relativePath)) {
      errors.push(
        `renderer durable-state primitive used outside the temporary boundary: ${relativePath} (${used.join(", ")})`,
      );
    }
  }
  for (const entry of rule.temporaryAllowedFiles ?? []) {
    if (!Number.isInteger(entry.issue) || entry.issue < 1 || !entry.reason) {
      errors.push(`invalid renderer mutation exception: ${entry.path}`);
    } else if (!observed.has(entry.path)) {
      errors.push(`resolved renderer mutation exception must be removed: ${entry.path}`);
    } else {
      notes.push(`temporary renderer mutation exception: ${entry.path} (issue #${entry.issue})`);
    }
  }
}

async function verifyWorkflowContract(root, policy, errors) {
  const contract = policy.workflowContract;
  if (!contract) {
    errors.push("workflowContract is required");
    return;
  }
  const workflowPath = join(root, contract.path ?? "");
  if (!(await exists(workflowPath))) {
    errors.push(`verification workflow is missing: ${contract.path}`);
    return;
  }
  const workflow = await readFile(workflowPath, "utf8");
  const requiredChecks = new Set(contract.requiredCheckNames ?? []);
  for (const packageEntry of policy.packages) {
    if (!requiredChecks.has(packageEntry.verificationJob)) {
      errors.push(
        `package ${packageEntry.manifest} references undeclared verification job: ${packageEntry.verificationJob}`,
      );
    }
  }
  for (const check of requiredChecks) {
    if (!workflow.includes(`name: ${check}`)) {
      errors.push(`verification workflow does not expose required check name: ${check}`);
    }
  }
  for (const snippet of contract.requiredSnippets ?? []) {
    if (!workflow.includes(snippet)) {
      errors.push(`verification workflow is missing required command: ${snippet}`);
    }
  }
}

async function verifyGeneratedContracts(root, policy, errors) {
  for (const contract of policy.generatedContracts ?? []) {
    if (!contract.source || !contract.generated) {
      errors.push(`invalid generated contract declaration: ${JSON.stringify(contract)}`);
      continue;
    }
    if (!(await exists(join(root, contract.source)))) {
      errors.push(`generated contract source is missing: ${contract.source}`);
    }
    if (!(await exists(join(root, contract.generated)))) {
      errors.push(`generated contract output is missing: ${contract.generated}`);
    }
  }
}

export async function verifyArchitecture({ root = DEFAULT_ROOT, policyPath = DEFAULT_POLICY } = {}) {
  const resolvedRoot = resolve(root);
  const policy = await readJson(join(resolvedRoot, policyPath));
  const errors = validatePolicyShape(policy);
  const notes = [];
  await verifyManifests(resolvedRoot, policy, errors);
  await verifyProviderImports(resolvedRoot, policy, errors, notes);
  await verifyMutationBoundary(resolvedRoot, policy, errors, notes);
  await verifyWorkflowContract(resolvedRoot, policy, errors);
  await verifyGeneratedContracts(resolvedRoot, policy, errors);
  if (errors.length) {
    throw new Error(`Architecture verification failed:\n- ${errors.join("\n- ")}`);
  }
  return {
    packages: policy.packages.length,
    generatedContracts: (policy.generatedContracts ?? []).length,
    manualVerificationRules: (policy.manualVerification ?? []).length,
    notes,
  };
}

async function main() {
  const summary = await verifyArchitecture();
  console.log(
    `Architecture verification passed: ${summary.packages} package manifests, ` +
      `${summary.generatedContracts} generated contracts, ` +
      `${summary.manualVerificationRules} manual verification rules.`,
  );
  for (const note of summary.notes) console.log(`NOTICE: ${note}`);
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(SCRIPT_PATH)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
