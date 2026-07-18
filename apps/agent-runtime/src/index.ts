import { pathToFileURL } from "node:url";
import { serverFromEnv } from "./server.js";

export * from "./ag-ui.js";
export * from "./analysis-validator.js";
export * from "./evidence.js";
export * from "./harness.js";
export * from "./os-client.js";
export * from "./prompt-compiler.js";
export * from "./skill-validator.js";
export * from "./tool-policy.js";
export * from "./tool-registry.js";
export * from "./types.js";

function main(): void {
  const server = serverFromEnv();
  const port = Number.parseInt(process.env.SHENNONG_AGENT_RUNTIME_PORT ?? "8002", 10);
  const host = process.env.SHENNONG_AGENT_RUNTIME_HOST ?? "0.0.0.0";
  server.listen(port, host, () => {
    process.stdout.write(`Shennong OS agent runtime listening on ${host}:${port}\n`);
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
