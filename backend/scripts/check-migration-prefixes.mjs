import { readdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const migrationsDirectory = path.resolve(scriptDirectory, "../migrations");
const migrationFiles = (await readdir(migrationsDirectory))
  .filter((file) => file.endsWith(".sql"))
  .sort();

const filesByPrefix = new Map();
const invalidFiles = [];
for (const file of migrationFiles) {
  const match = /^(\d{3})_.+\.sql$/.exec(file);
  if (!match) {
    invalidFiles.push(file);
    continue;
  }
  const files = filesByPrefix.get(match[1]) ?? [];
  files.push(file);
  filesByPrefix.set(match[1], files);
}

const collisions = [...filesByPrefix.entries()].filter(([, files]) => files.length > 1);
if (invalidFiles.length || collisions.length) {
  console.error("Invalid backend migration numbering:");
  for (const file of invalidFiles) console.error(`- ${file} does not start with a three-digit prefix`);
  for (const [prefix, files] of collisions) console.error(`- prefix ${prefix} is used by: ${files.join(", ")}`);
  process.exit(1);
}
console.log(`Validated ${migrationFiles.length} uniquely numbered migrations.`);
