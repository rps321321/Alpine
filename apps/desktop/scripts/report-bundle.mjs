import { readdir, stat } from "node:fs/promises";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../dist/client/", import.meta.url));

async function files(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map(async (entry) => {
      const path = join(directory, entry.name);
      return entry.isDirectory() ? files(path) : [path];
    }),
  );
  return nested.flat();
}

const rows = await Promise.all(
  (await files(root)).map(async (path) => ({
    asset: relative(root, path).replaceAll("\\", "/"),
    bytes: (await stat(path)).size,
  })),
);

rows.sort((left, right) => right.bytes - left.bytes);
console.table(rows);
const total = rows.reduce((sum, row) => sum + row.bytes, 0);
console.log(`Total client assets: ${(total / 1024).toFixed(1)} KiB`);
