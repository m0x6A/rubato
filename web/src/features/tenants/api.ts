import type { InviteRequest, MemberView, Role, TenancyRefusal } from "../../protocol.ts";
import { ApiError, failureText, request } from "../../shared/api.ts";
import { inTenant } from "../../shared/workspace.ts";

export const membersApi = {
  /** Who is in this page's tenant. Every member may read it. */
  list: () => request<MemberView[]>("GET", inTenant("/api/members")),
  /** An admin invites a GitHub login; the answer is the account that was invited. */
  invite: (r: InviteRequest) => request<MemberView>("POST", inTenant("/api/members"), r),
  setRole: (githubId: number, role: Role) => request<void>("PUT", inTenant(`/api/members/${githubId}`), { role }),
  remove: (githubId: number) => request<void>("DELETE", inTenant(`/api/members/${githubId}`)),
};

/** The reason of a tenancy refusal, when the failure carries one. */
export function refusalOf(e: unknown): TenancyRefusal | null {
  if (!(e instanceof ApiError)) {
    return null;
  }
  try {
    const body = JSON.parse(e.message) as Partial<TenancyRefusal>;
    return typeof body.reason === "string" ? (body as TenancyRefusal) : null;
  } catch {
    return null;
  }
}

const WORDS: Record<string, string> = {
  unknown_login: "GitHub has no account with that login.",
  already_member: "That person is already a member.",
  last_admin: "A tenant keeps at least one admin. Make someone else an admin first.",
  not_a_member: "That person is not a member any more.",
  not_admin: "Only an admin of the tenant can do that.",
  lookup_failed: "GitHub could not be asked who that login is. Nothing was changed; try again.",
};

/** What to tell the person about a failed members call, in words. */
export function refusalText(e: unknown): string {
  const r = refusalOf(e);
  return r ? (WORDS[r.reason] ?? r.reason) : failureText(e);
}
