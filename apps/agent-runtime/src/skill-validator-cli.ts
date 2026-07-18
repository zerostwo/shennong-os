import { resolve } from "node:path";
import { validateSkillsRoot } from "./skill-validator.js";

const root = resolve(process.argv[2] ?? "../../skills");
const results = await validateSkillsRoot(root);
for (const result of results) {
  process.stdout.write(`${result.valid ? "PASS" : "FAIL"} ${result.directory}\n`);
  for (const finding of result.findings) {
    process.stdout.write(`  ${finding.severity.toUpperCase()} ${finding.code}: ${finding.message}\n`);
  }
  if (result.expectedDigest) process.stdout.write(`  DIGEST ${result.expectedDigest}\n`);
}
if (!results.length || results.some(({ valid }) => !valid)) process.exitCode = 1;
