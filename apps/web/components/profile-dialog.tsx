"use client";

import { ChangeEvent, FormEvent, useEffect, useRef, useState } from "react";
import Image from "next/image";
import { Camera, UserRound } from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { updateProfile } from "@/lib/api/adapter";
import type { AuthSession } from "@/lib/auth-session";

export function ProfileDialog({ open, session, onOpenChange, onSaved }: { open: boolean; session: AuthSession | null; onOpenChange: (open: boolean) => void; onSaved: (session: AuthSession) => void }) {
  const [displayName, setDisplayName] = useState("");
  const [username, setUsername] = useState("");
  const [avatarUrl, setAvatarUrl] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!open) return;
    setDisplayName(session?.display_name || "");
    setUsername(session?.username || "");
    setAvatarUrl(session?.avatar_url || "");
    setError("");
  }, [open, session]);

  function chooseAvatar(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    if (!file) return;
    if (file.size > 500_000 || !["image/png", "image/jpeg", "image/webp"].includes(file.type)) {
      setError("Choose a PNG, JPEG, or WebP image under 500 KB.");
      return;
    }
    const reader = new FileReader();
    reader.onload = () => { setAvatarUrl(String(reader.result)); setError(""); };
    reader.onerror = () => setError("The image could not be read.");
    reader.readAsDataURL(file);
  }

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusy(true);
    setError("");
    try {
      const updated = await updateProfile({ display_name: displayName.trim(), username: username.trim(), avatar_url: avatarUrl });
      onSaved(updated);
      onOpenChange(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Profile could not be saved");
    } finally { setBusy(false); }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="profile-dialog">
        <DialogTitle>Edit profile</DialogTitle>
        <DialogDescription>Update how you appear throughout Shennong.</DialogDescription>
        <form onSubmit={save}>
          <div className="profile-avatar-editor">
            <span className="profile-avatar-preview">{avatarUrl ? <Image src={avatarUrl} alt="Profile avatar preview" width={112} height={112} unoptimized /> : <UserRound />}</span>
            <button type="button" onClick={() => fileInput.current?.click()} aria-label="Choose profile avatar"><Camera /></button>
            <input ref={fileInput} className="sr-only" type="file" accept="image/png,image/jpeg,image/webp" onChange={chooseAvatar} />
          </div>
          <label>Display name<input value={displayName} onChange={(event) => setDisplayName(event.target.value)} autoFocus required maxLength={128} /></label>
          <label htmlFor="profile-username">Username<span className="profile-username-input"><b>@</b><input id="profile-username" aria-label="Username" value={username} onChange={(event) => setUsername(event.target.value.toLowerCase())} required minLength={3} maxLength={32} pattern={"[a-z0-9][a-z0-9._\\-]{2,31}"} /></span></label>
          {error ? <div className="settings-error" role="alert">{error}</div> : null}
          <div className="profile-dialog-actions"><button type="button" className="settings-secondary" onClick={() => onOpenChange(false)}>Cancel</button><button className="settings-primary" disabled={busy}>{busy ? "Saving…" : "Save profile"}</button></div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
