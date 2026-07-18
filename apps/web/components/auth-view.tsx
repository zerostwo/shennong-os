"use client";

import Link from "next/link";
import { FormEvent, useEffect, useState } from "react";
import { ArrowLeft, Beaker, CheckCircle2 } from "lucide-react";
import {
  getPublicConfig,
  getSetupStatus,
  registerUser,
  setupAdmin,
  signIn,
  ShennongApiError,
} from "@/lib/api/adapter";
import { safeInternalReturnTo } from "@/lib/safe-return-to";

type AuthStep = "loading" | "setup" | "signin" | "register" | "done";

export function AuthView() {
  const [step, setStep] = useState<AuthStep>("loading");
  const [registrationEnabled, setRegistrationEnabled] = useState(false);
  const [inviteRequired, setInviteRequired] = useState(false);
  const [doneRole, setDoneRole] = useState("");
  const [returnTo, setReturnTo] = useState("/projects");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [expired, setExpired] = useState(false);

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    setExpired(params.get("reason") === "session-expired");
    setReturnTo(safeInternalReturnTo(params.get("returnTo")));
    void Promise.allSettled([getSetupStatus(), getPublicConfig()]).then(([setup, config]) => {
      const setupValue = setup.status === "fulfilled" ? setup.value : { needs_setup: false };
      const configValue = config.status === "fulfilled" ? config.value : {};
      const registrationIsOpen = configValue.registration_mode === "open" || configValue.registration_mode === "invite_only" || configValue.registration_enabled === true;
      setRegistrationEnabled(registrationIsOpen);
      setInviteRequired(configValue.invite_required === true);
      if (setupValue.needs_setup) setStep("setup");
      else setStep(params.get("mode") === "register" && registrationIsOpen ? "register" : "signin");
    });
  }, []);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError("");
    setBusy(true);
    const form = new FormData(event.currentTarget);
    const displayName = String(form.get("display_name") ?? "");
    const email = String(form.get("email") ?? "");
    const password = String(form.get("password") ?? "");
    try {
      if (step === "setup") {
        const result = await setupAdmin(displayName, email, password, String(form.get("bootstrap_token") ?? ""));
        if (!result.authenticated) throw new Error("Administrator session was not established");
        setDoneRole(result.role || "admin");
        setStep("done");
      } else if (step === "register") {
        if (password !== String(form.get("password_confirm") ?? "")) throw new Error("Passwords do not match");
        const result = await registerUser(displayName, email, password, String(form.get("invite_code") ?? ""));
        if (!result.authenticated) throw new Error("Account session was not established");
        setDoneRole(result.role || "user");
        setStep("done");
      } else if (step === "signin") {
        const result = await signIn(email, password);
        if (result.authenticated) {
          setDoneRole(result.role ?? "");
          setStep("done");
        } else throw new Error("Authentication was not established");
      }
    } catch (reason) {
      setError(reason instanceof ShennongApiError ? reason.message : reason instanceof Error ? reason.message : "Authentication failed");
    } finally { setBusy(false); }
  };

  if (step === "loading") return <main className="auth-screen"><div className="auth-card" role="status">Loading secure sign in…</div></main>;
  const title = step === "setup" ? "Create the administrator" : step === "signin" ? "Welcome back" : "Create your account";
  const description = step === "setup"
    ? "Create the first administrator using the one-time token generated during deployment."
    : step === "signin"
      ? "Sign in to use Agent Chat and your private data."
      : inviteRequired
        ? "Early access registration requires an invitation code."
        : "Create an account to start a governed biomedical analysis workspace.";
  return (
    <main className="auth-screen">
      <Link className="auth-brand" href="/"><Beaker />Shennong</Link>
      <div className="auth-card">
        {step === "done" ? (
          <>
            <div className="auth-success"><CheckCircle2 /><strong>Authenticated</strong><span>Your secure session is active.</span></div>
            <h1>{doneRole === "admin" ? "Administrator ready" : "Welcome to Shennong"}</h1>
            <p>Continue to Agent Chat or browse governed biomedical Resources.</p>
            <div className="dialog-actions"><Link className="outline-button" href="/resources">Open Resources</Link><Link className="primary-button" href={returnTo}>Continue</Link></div>
            {doneRole === "admin" ? <Link className="auth-admin-link" href="/admin/invites">Open Admin center</Link> : null}
          </>
        ) : (
          <>
            {expired ? <div className="form-error-summary" role="alert"><strong>Session expired</strong><span>Sign in again to continue.</span></div> : null}
            <h1>{title}</h1>
            <p>{description}</p>
            <form onSubmit={(event) => void submit(event)}>
              {step === "setup" || step === "signin" || step === "register" ? (
                <>
                  {step !== "signin" ? <label>Display name<input name="display_name" required autoComplete="name" /></label> : null}
                  <label>Email<input name="email" type="email" required autoComplete="email" /></label>
                  <label>Password<input name="password" type="password" required minLength={12} autoComplete={step === "signin" ? "current-password" : "new-password"} /></label>
                  {step === "setup" ? <label>Bootstrap token<input name="bootstrap_token" type="password" required autoComplete="off" spellCheck={false} /></label> : null}
                  {step === "register" ? <label>Confirm password<input name="password_confirm" type="password" required minLength={12} autoComplete="new-password" /></label> : null}
                  {step === "register" && inviteRequired ? <label>Invitation code<input name="invite_code" required autoComplete="off" spellCheck={false} /></label> : null}
                </>
              ) : null}
              {error ? <p className="form-error" role="alert">{error}</p> : null}
              <button className="primary-button" type="submit" disabled={busy}>{busy ? "Working…" : step === "setup" ? "Create administrator" : step === "signin" ? "Sign in" : "Create account"}</button>
            </form>
            {step === "signin" && registrationEnabled ? <div className="auth-mode-switch"><span>{inviteRequired ? "Have an invitation?" : "New to Shennong?"}</span><button onClick={() => { setError(""); setStep("register"); }}>Create account</button></div> : null}
            {step === "register" ? <div className="auth-mode-switch"><span>Already have an account?</span><button onClick={() => { setError(""); setStep("signin"); }}>Sign in</button></div> : null}
            {step === "signin" || step === "register" ? <Link className="auth-public-link" href="/resources"><ArrowLeft />Browse public Resources without an account</Link> : null}
          </>
        )}
      </div>
    </main>
  );
}
