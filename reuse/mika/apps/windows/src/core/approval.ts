import type { ApprovalInfo } from "./state";

/** Match the approval card's delay and explicit pointer rule on every companion surface. */
export const ALLOW_ARM_MS = 600;
export interface ApprovalClick { isTrusted: boolean; detail: number }
export const isPointerAction = (click: ApprovalClick) => click.isTrusted && click.detail > 0;

/** The agent of MIKA a gate request is for (`_agent: "agent:<id>"`, set by Rust, never by the relay), or null. */
export function agentOf(payload: { _agent?: string }): string | null {
  const tag = payload._agent;
  return typeof tag === "string" && /^agent:[a-z0-9][a-z0-9_-]*$/.test(tag) ? tag.slice(6) : null;
}

export function approvalDecisionReady(
  request: ApprovalInfo | null, requestId: string, shownAt: number, now: number, click: ApprovalClick,
) {
  return request !== null && request.requestId === requestId && !request.truncated
    && now - shownAt >= ALLOW_ARM_MS && isPointerAction(click);
}
