// Typed wrappers around the app's commands and events.

import type { Intent } from "./body/intent";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Lane = "planner" | "quick";
export type Outcome = "done" | "halted" | "failed" | "step_limit" | "timed_out";
export type Decision = "approved" | "denied" | "cancelled";

export type AgentEvent =
  | { type: "task_started"; task_id: string; goal: string }
  | { type: "thinking"; task_id: string }
  | { type: "text_delta"; task_id: string; lane: Lane; text: string }
  | { type: "text_done"; task_id: string; lane: Lane }
  | { type: "tool_started"; task_id: string; call_id: string; tool: string; summary: string; tier: number }
  | { type: "tool_output"; task_id: string; call_id: string; line: string }
  | { type: "tool_finished"; task_id: string; call_id: string; tool: string; ok: boolean; summary: string }
  | { type: "approval_resolved"; id: string; decision: Decision }
  | { type: "task_finished"; task_id: string; outcome: Outcome; message: string }
  | { type: "notice"; text: string }
  | { type: "trace_saved"; task_id: string }
  | { type: "offer"; id: string; label: string };

export interface ApprovalRequest {
  id: string;
  task_id: string;
  tier: number;
  tool: string;
  summary: string;
  reason: string;
  detail: string;
  countdown_ms: number | null;
  /** An email about to be sent: the send card shows it in full and lets the user edit it. */
  draft?: MailDraft;
}

export interface MailDraft {
  to: string;
  cc: string;
  subject: string;
  body: string;
  reply_to?: string;
}

/** A tap on the shoulder: a meeting soon, important mail, or the morning brief offer. */
export interface NudgePayload {
  id: string;
  kind: "meeting" | "mail" | "brief";
  stage: "small" | "full";
  text: string;
  actions: { id: string; label: string }[];
}

/** Text grabbed with Ctrl+Alt+A, as the chat box shows it. */
export interface SelectionPreview {
  chars: number;
  preview: string;
  app: string;
}

export interface Platform {
  id: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Bootstrap {
  color: string;
  wander: boolean;
  platform: "windows" | "macos" | "linux";
  demo: boolean;
  voice_backend: "system" | "whisper_api" | "off";
  windows: Platform[];
}

export interface HitRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export const api = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  sendMessage: (text: string, selection = false) => invoke<void>("send_message", { text, selection }),
  dropSelection: () => invoke<void>("drop_selection"),
  openAnswer: (id: string) => invoke<string>("open_answer", { id }),
  warmUp: () => invoke<void>("warm_up"),
  halt: () => invoke<boolean>("halt"),
  answerApproval: (id: string, approved: boolean, draft: MailDraft | null = null) => invoke<void>("answer_approval", { id, approved, draft }),
  undoSend: (id: string) => invoke<void>("undo_send", { id }),
  nudgeAction: (id: string, action: string) => invoke<string | null>("nudge_action", { id, action }),
  duckArrived: (id: string) => invoke<void>("duck_arrived", { id }),
  setHitRects: (rects: HitRect[]) => invoke<void>("set_hit_rects", { rects }),
  setCapture: (on: boolean) => invoke<void>("set_capture", { on }),
  rateTask: (taskId: string, good: boolean) => invoke<void>("rate_task", { taskId, good }),
  openSettings: () => invoke<void>("open_settings"),
  voiceStart: () => invoke<"system" | "recording">("voice_start"),
  voiceStop: () => invoke<string | null>("voice_stop"),
};

export interface Events {
  agent: AgentEvent;
  approval: ApprovalRequest;
  busy: boolean;
  "desktop:windows": Platform[];
  "duck:move": { id: string; x: number; y: number; purpose: "approach" | "act" };
  "duck:act": { kind: "peck" | "type" | "look" };
  "duck:point": { x: number; y: number; label: string };
  reminder: { text: string; late: boolean };
  "duck:intent": { intent: Intent; window: Platform | null; cursor: { x: number; y: number } | null };
  settings: { color: string; wander: boolean; demo: boolean };
  "chat:open": { voice: boolean; selection?: SelectionPreview | null };
  "wander:toggle": null;
  undo: { id: string; secs: number };
  nudge: NudgePayload;
  "nudge:chime": null;
  "undo:done": { id: string; undone: boolean };
}

export function on<K extends keyof Events>(name: K, handler: (payload: Events[K]) => void): Promise<UnlistenFn> {
  return listen<Events[K]>(name, (e) => handler(e.payload));
}
