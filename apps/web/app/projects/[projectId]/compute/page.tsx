import { ComputeView } from "@/components/compute-view";

export default async function ProjectComputePage({ params }: { params: Promise<{ projectId: string }> }) {
  const { projectId } = await params;
  return <ComputeView projectId={projectId} />;
}
