export type TaskStatus =
  | 'pending'
  | 'running'
  | 'needs_approval'
  | 'done'
  | 'failed'
  | 'cancelled'
  | 'scheduled'
  | 'paused'

export type EventKind =
  | 'instruction'
  | 'plan'
  | 'thinking'
  | 'tool_call'
  | 'tool_result'
  | 'subagent_spawn'
  | 'message'
  | 'artifact'
  | 'approval_request'
  | 'approval_response'
  | 'checkpoint'
  | 'error'
  | 'system'

export type TocIcon =
  | 'message'
  | 'plan'
  | 'thinking'
  | 'tool'
  | 'artifact'
  | 'approval'
  | 'checkpoint'
  | 'error'
  | 'fork'

export interface TokenUsage {
  input_tokens: number
  output_tokens: number
  cost_usd_cents: number
}

export interface AgentPersona {
  vertical_id?: string | null
  system_prompt: string
  model_id?: string | null
  provider_id?: string | null
}

export interface CronSchedule {
  expr: string
  timezone: string
  next_run?: string | null
  last_run?: string | null
  enabled: boolean
}

export interface Task {
  id: string
  owner_user_id: string
  primary_device_id: string
  title: string
  vertical?: string | null
  status: TaskStatus
  parent_task_id?: string | null
  persona_stack: AgentPersona[]
  schedule?: CronSchedule | null
  created_at: string
  updated_at: string
}

export type TaskActor =
  | { type: 'user'; user_id: string; device_id: string }
  | { type: 'agent'; model_id: string; provider_id: string }
  | { type: 'tool'; tool_name: string }
  | { type: 'system' }

export interface TaskEvent {
  id: string
  task_id: string
  parent_event_id?: string | null
  kind: EventKind
  actor: TaskActor
  payload: Record<string, unknown>
  collapsed_by_default: boolean
  streaming: boolean
  created_at: string
  duration_ms?: number | null
  cost_tokens?: TokenUsage | null
  turn_id?: string | null
  toc_label?: string | null
  toc_icon?: TocIcon | null
  anchor_slug: string
}

export interface TaskTurn {
  id: string
  task_id: string
  instruction_event_id: string
  label: string
  started_at: string
  ended_at?: string | null
  status: 'running' | 'done' | 'cancelled' | 'failed'
  artifact_count: number
  approval_count: number
  error_count: number
  cost_tokens: TokenUsage
  sub_task_ids: string[]
}

export interface VerticalMeta {
  id: string
  display_name_key: string
  description_key: string
  icon: string
  category: string
  order: number
  task_category?: string | null
}

export interface TaskListParams {
  owner_user_id?: string
  status?: TaskStatus
  vertical?: string
  cursor?: string
  limit?: number
}

export interface TaskListResponse {
  tasks: Task[]
  next_cursor?: string | null
}

export interface CreateTaskRequest {
  title: string
  vertical?: string
  instruction?: string
  owner_user_id?: string
  device_id?: string
}

export interface CreateTaskResponse {
  task: Task
  event: TaskEvent
}

export interface TaskEventsResponse {
  events: TaskEvent[]
}

export interface TaskTurnsResponse {
  turns: TaskTurn[]
}

export interface VerticalListResponse {
  verticals: VerticalMeta[]
}

export const TASK_STATUS_GROUPS: { key: string; statuses: TaskStatus[] }[] = [
  { key: 'running', statuses: ['running', 'pending'] },
  { key: 'approval', statuses: ['needs_approval'] },
  { key: 'scheduled', statuses: ['scheduled', 'paused'] },
  { key: 'done', statuses: ['done', 'failed', 'cancelled'] }
]
