import { redirect } from "next/navigation";
export default async function Page({
  params,
}: {
  params: Promise<{ userId: string }>;
}) {
  await params;
  redirect("/admin/invites");
}
