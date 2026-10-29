import { useCallback, useEffect, useRef, useState } from "react";
import { checkAppUpdate } from "../lib/updates";
import type { UpdateCheckResult } from "../types/updates";

export function useUpdateNotice() {
  const [result, setResult] = useState<UpdateCheckResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const [dialogRequested, setDialogRequested] = useState(false);
  const [cooldownUntil, setCooldownUntil] = useState(0);
  const [cooldownSeconds, setCooldownSeconds] = useState(0);
  const session = useRef(0);
  const mounted = useRef(false);
  const pending = useRef<Promise<UpdateCheckResult | null> | null>(null);

  const check = useCallback((force: boolean): Promise<UpdateCheckResult | null> => {
    if (pending.current) return pending.current;
    const requestSession = session.current;
    setBusy(true);
    setFailed(false);
    const request = checkAppUpdate(force).then(next => {
      if (!mounted.current || session.current !== requestSession) return null;
      if (next) {
        setResult(next);
        if (next.status === "updateAvailable" && next.release) setDialogRequested(true);
        else setDialogRequested(false);
        if (!next.fromCache || force) setCooldownUntil(Date.now() + 30_000);
      }
      return next;
    }).catch(() => {
      if (mounted.current && session.current === requestSession) {
        setFailed(true);
        setCooldownUntil(Date.now() + 30_000);
      }
      return null;
    }).finally(() => {
      if (pending.current === request) pending.current = null;
      if (mounted.current && session.current === requestSession) setBusy(false);
    });
    pending.current = request;
    return request;
  }, []);

  useEffect(() => {
    mounted.current = true;
    session.current += 1;
    // A remount subscribes to the shared native request with its own lifetime guard.
    pending.current = null;
    void check(false);
    return () => { mounted.current = false; session.current += 1; };
  }, [check]);

  useEffect(() => {
    const remaining = () => Math.max(0, Math.ceil((cooldownUntil - Date.now()) / 1000));
    setCooldownSeconds(remaining());
    if (!remaining()) return;
    const timer = window.setInterval(() => {
      const seconds = remaining();
      setCooldownSeconds(seconds);
      if (!seconds) window.clearInterval(timer);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [cooldownUntil]);

  return {
    result, busy, failed, cooldownSeconds, dialogRequested,
    release: result?.status === "updateAvailable" ? result.release : null,
    check,
    open: () => setDialogRequested(true),
    dismiss: () => setDialogRequested(false),
  };
}
