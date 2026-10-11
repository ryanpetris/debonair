import { useEffect, useState } from "react";
import type { Action, ActionResult, AppState, DesktopApi } from "../shared/state.ts";

declare global {
  interface Window {
    debonair: DesktopApi;
  }
}

export const api = (): DesktopApi => window.debonair;

export function useAppState(): AppState | null {
  const [state, setState] = useState<AppState | null>(null);
  useEffect(() => {
    const stop = api().onState(setState);
    void api().state().then((initial) => setState((current) => current ?? initial));
    return stop;
  }, []);
  return state;
}

export type Feedback = { ok: boolean; message: string } | null;

/** Runs an action and turns its result into feedback for the control that started it. */
export async function perform(action: Action, success: string): Promise<Feedback> {
  const result: ActionResult = await api().act(action);
  return result.ok ? (success ? { ok: true, message: success } : null) : { ok: false, message: result.message };
}
