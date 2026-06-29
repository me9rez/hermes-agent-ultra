import type {
  CreateTaskRequest,
  CreateTaskResponse,
  Task,
  TaskEventsResponse,
  TaskListParams,
  TaskListResponse,
  TaskTurnsResponse,
  VerticalListResponse
} from '@/types/task'

function api<T>(request: { path: string; method?: string; body?: unknown }): Promise<T> {
  return window.hermesDesktop.api<T>(request)
}

function buildQuery(params: Record<string, string | number | undefined | null>): string {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === '') continue
    search.set(key, String(value))
  }
  const qs = search.toString()
  return qs ? `?${qs}` : ''
}

export async function listTasks(params: TaskListParams = {}): Promise<TaskListResponse> {
  return api<TaskListResponse>({
    path: `/api/tasks${buildQuery({
      owner_user_id: params.owner_user_id,
      status: params.status,
      vertical: params.vertical,
      cursor: params.cursor,
      limit: params.limit ?? 50
    })}`
  })
}

export async function getTask(taskId: string): Promise<Task> {
  return api<Task>({ path: `/api/tasks/${encodeURIComponent(taskId)}` })
}

export async function createTask(body: CreateTaskRequest): Promise<CreateTaskResponse> {
  return api<CreateTaskResponse>({
    path: '/api/tasks',
    method: 'POST',
    body
  })
}

export async function deleteTask(taskId: string): Promise<void> {
  await api<null>({ path: `/api/tasks/${encodeURIComponent(taskId)}`, method: 'DELETE' })
}

export async function listTaskEvents(taskId: string): Promise<TaskEventsResponse> {
  return api<TaskEventsResponse>({ path: `/api/tasks/${encodeURIComponent(taskId)}/events` })
}

export async function listTaskTurns(taskId: string): Promise<TaskTurnsResponse> {
  return api<TaskTurnsResponse>({ path: `/api/tasks/${encodeURIComponent(taskId)}/turns` })
}

export async function continueTask(taskId: string, instruction: string): Promise<unknown> {
  return api({
    path: `/api/tasks/${encodeURIComponent(taskId)}/continue`,
    method: 'POST',
    body: { instruction }
  })
}

export async function cancelTask(taskId: string): Promise<{ cancelled: boolean }> {
  return api<{ cancelled: boolean }>({
    path: `/api/tasks/${encodeURIComponent(taskId)}/cancel`,
    method: 'POST'
  })
}

export async function approveTask(
  taskId: string,
  eventId: string,
  approved: boolean,
  reason?: string
): Promise<unknown> {
  return api({
    path: `/api/tasks/${encodeURIComponent(taskId)}/approve`,
    method: 'POST',
    body: { event_id: eventId, approved, reason }
  })
}

export async function listVerticals(params?: { category?: string; search?: string }): Promise<VerticalListResponse> {
  return api<VerticalListResponse>({
    path: `/api/verticals${buildQuery({
      category: params?.category,
      search: params?.search
    })}`
  })
}

export async function getVertical(verticalId: string): Promise<unknown> {
  return api({ path: `/api/verticals/${encodeURIComponent(verticalId)}` })
}
