import { invoke } from "@tauri-apps/api/core";
import type { AgentRequest, AgentResponse, ResponseEnvelope } from "./types";

export async function requestAgent(request: AgentRequest): Promise<AgentResponse> {
  const envelope = await invoke<ResponseEnvelope>("agent_request", { request });
  return envelope.body;
}

export function requireSuccessfulResponse(response: AgentResponse): AgentResponse {
  if (response.type === "error") {
    throw new Error(response.error.message);
  }
  return response;
}

