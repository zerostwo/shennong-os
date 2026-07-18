import type { GovernedToolDefinition } from "./tool-registry.js";
import type { RunRequest } from "./types.js";

export interface LocalPolicyDecision {
  allowed: boolean;
  reason?: string;
}

export class ToolPolicyEngine {
  check(run: RunRequest, definition: GovernedToolDefinition): LocalPolicyDecision {
    if (!definition.profiles.includes(run.toolProfile)) {
      return { allowed: false, reason: "tool_not_in_profile" };
    }
    if (definition.requiresProject && !run.scope.projectId) {
      return { allowed: false, reason: "project_scope_required" };
    }
    if (definition.risk === "admin" && run.scope.role !== "admin") {
      return { allowed: false, reason: "admin_required" };
    }
    if (
      definition.mayUsePrivateData &&
      run.scope.providerDataPolicy === "public_only" &&
      run.context &&
      (run.context.project || run.context.attachments?.length || run.context.artifacts?.length)
    ) {
      return { allowed: false, reason: "private_context_not_allowed_by_provider" };
    }
    const declared = new Set(
      (run.context?.selectedSkills ?? []).flatMap(({ permissions }) => permissions.tools),
    );
    const core = new Set(["skill.load", "plan.propose", "plan.update", "analysis.validate"]);
    if (!declared.has(definition.name) && !core.has(definition.name)) {
      return { allowed: false, reason: "tool_not_declared_by_selected_skills" };
    }
    return { allowed: true };
  }
}
