import { redirect } from "next/navigation";

export default async function ProjectUploadPage({ params }: { params: Promise<{ projectId: string }> }) {
  const { projectId } = await params;
  redirect(`/projects/${encodeURIComponent(projectId)}/uploads/new`);
}
