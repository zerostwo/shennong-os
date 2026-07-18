"use client";

import Link from "next/link";
import { FormEvent, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { useQueryClient } from "@tanstack/react-query";
import { FileIcon, FolderLock, MessageSquare, UploadCloud } from "lucide-react";
import { buildProjectUploadPrompt, registerProjectFiles } from "@/lib/project-upload";
import { AppShell, TopBar } from "./app-shell";
import styles from "./project-ui.module.css";

const MAX_UPLOAD_BYTES = 50 * 1024 * 1024 * 1024;

export function UploadView({ projectId }: { projectId: string }) {
  const router = useRouter();
  const queryClient = useQueryClient();
  const submitting = useRef(false);
  const [files, setFiles] = useState<File[]>([]);
  const [background, setBackground] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  function selectFiles(selected: File[]) {
    setError("");
    if (selected.length > 20) {
      setFiles([]);
      setError("Upload no more than 20 related files at once.");
      return;
    }
    const invalid = selected.find((file) => file.size === 0 || file.size > MAX_UPLOAD_BYTES);
    if (invalid) {
      setFiles([]);
      setError(`${invalid.name} must be non-empty and no larger than 50 GiB.`);
      return;
    }
    setFiles(selected);
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (submitting.current) return;
    if (!files.length) {
      setError("Select at least one file.");
      return;
    }
    submitting.current = true;
    setBusy(true);
    setError("");
    try {
      const result = await registerProjectFiles(projectId, files, background);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["projects", projectId, "context-pack"] }),
        queryClient.invalidateQueries({ queryKey: ["projects", projectId, "resources"] }),
      ]);
      const prompt = buildProjectUploadPrompt(result, background);
      router.push(`/projects/${encodeURIComponent(projectId)}/chat?handoff=${encodeURIComponent(prompt)}`);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Upload failed");
    } finally {
      submitting.current = false;
      setBusy(false);
    }
  }

  return (
    <AppShell active="projects">
      <TopBar title="Add data with the Agent" description="Upload files now, then let the Agent inspect and organize them in Project chat." search={false} />
      <main className={styles.uploadLayout}>
        <div className={styles.uploadIntro}>
          <h1>Add experimental files</h1>
          <p>You only need to choose the files and share any useful background. Shennong registers a private, immutable Project resource before the Agent sees its durable reference.</p>
        </div>
        <form className={styles.uploadForm} onSubmit={submit}>
          {error ? <div className="form-error-summary" role="alert">{error}</div> : null}
          <label className={styles.dropzone}>
            <UploadCloud />
            <span><strong>{files.length ? "Choose different files" : "Choose files"}</strong><small>One or more files, up to 50 GiB each</small></span>
            <input type="file" multiple hidden onChange={(event) => selectFiles(Array.from(event.target.files ?? []))} />
          </label>
          {files.length ? <div className={styles.fileList}>{files.map((file) => <div className={styles.fileRow} key={`${file.name}-${file.lastModified}`}><FileIcon /><span>{file.name}</span><small>{formatBytes(file.size)}</small></div>)}</div> : null}
          <label className={styles.backgroundField}>
            Optional background
            <textarea value={background} maxLength={4000} onChange={(event) => setBackground(event.target.value)} placeholder="For example: paired tumor and normal RNA-seq from 12 patients, hg38, stranded library." />
          </label>
          <div className={styles.privacyNote}><FolderLock /><span>Files stay private to this Project. Actor, owner, visibility, and Project identity are enforced by the server.</span></div>
          <div className={styles.uploadActions}>
            <Link className="outline-button" href={`/projects/${encodeURIComponent(projectId)}`}>Cancel</Link>
            <button className="primary-button" disabled={busy}><MessageSquare />{busy ? "Uploading and registering…" : "Continue in Project chat"}</button>
          </div>
        </form>
      </main>
    </AppShell>
  );
}

function formatBytes(value: number) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) { size /= 1024; index += 1; }
  return `${size.toFixed(size >= 10 ? 1 : 2)} ${units[index]}`;
}
