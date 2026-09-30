import { invoke } from "@tauri-apps/api/core";

export type PrivacyScope =
  | { kind: "forget_term"; term: string }
  | { kind: "clear_learned" };
export interface PrivacyRequest {
  operation_id: string;
  scope: PrivacyScope;
  expected_epoch: number;
}
export interface PrivacyOperation {
  operation_id: string;
  scope: PrivacyScope["kind"];
  expected_epoch: number;
  epoch: number;
  state: "accepted" | "processing" | "partial_failure" | "completed";
  domain_receipts: {
    integration: boolean;
    personalization: boolean;
    readers: boolean;
    legacy_memory: boolean;
  };
  failure: string | null;
  coverage: "primary_only" | "all_domains" | "legacy_coverage_unresolved";
}
export interface PrivacyStatus {
  epoch: number;
  operations: PrivacyOperation[];
}
export const getPrivacyStatus = () =>
  invoke<PrivacyStatus>("knowledge_request", {
    action: "privacy_status",
    payload: {},
  });
export const beginPrivacyOperation = (payload: PrivacyRequest) =>
  invoke<PrivacyOperation>("knowledge_request", {
    action: "privacy_begin",
    payload,
  });
