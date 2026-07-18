import { ChatView } from "@/components/chat-view";

export default async function ProjectChatPage({
  params,
  searchParams,
}: {
  params: Promise<{ projectId: string }>;
  searchParams: Promise<{ handoff?: string | string[] }>;
}) {
  const { projectId } = await params;
  const query = await searchParams;
  const initialPrompt = typeof query.handoff === "string" ? query.handoff : undefined;
  return <ChatView projectId={projectId} initialPrompt={initialPrompt} />;
}
