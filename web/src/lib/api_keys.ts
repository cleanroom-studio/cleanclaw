// Stub agent apikeys endpoint — backend integration comes next phase.

export interface ApiKeyInfo {
  id: string;
  type: string;
  key_prefix: string;
  name: string;
}

export interface ApiKeyCreate {
  name?: string;
  type: string;
}
