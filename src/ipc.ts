// Typed wrappers around the app's commands and events.

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
  | { type: "notice"; text: string };

export interface ApprovalRequest {
  id: string;
  task_id: string;
  tier: number;
  tool: string;
  summary: string;
  reason: string;
  detail: string;
  countdown_ms: number | null;
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
  sendMessage: (text: string) => invoke<void>("send_message", { text }),
  warmUp: () => invoke<void>("warm_up"),
  halt: () => invoke<boolean>("halt"),
  answerApproval: (id: string, approved: boolean) => invoke<void>("answer_approval", { id, approved }),
  duckArrived: (id: string) => invoke<void>("duck_arrived", { id }),
  setHitRects: (rects: HitRect[]) => invoke<void>("set_hit_rects", { rects }),
  setCapture: (on: boolean) => invoke<void>("set_capture", { on }),
  playSnapshot: () => invoke<string>("play_snapshot"),
  playInput: (on: boolean) => invoke<void>("play_input", { on }),
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
  settings: { color: string; wander: boolean; demo: boolean };
  "chat:open": { voice: boolean };
  "wander:toggle": null;
  "play:start": { autoplay: boolean; weapon?: string | null; target?: HitRect | null };
  "play:stop": null;
}

export function on<K extends keyof Events>(name: K, handler: (payload: Events[K]) => void): Promise<UnlistenFn> {
  return listen<Events[K]>(name, (e) => handler(e.payload));
}
