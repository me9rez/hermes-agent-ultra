import { atom, computed } from 'nanostores'

import { approveTask } from '@/lib/task-api'
import type { TaskEvent } from '@/types/task'
import { eventApprovalSummary } from '@/lib/task-event-utils'

export interface ApprovalQueueItem {
  taskId: string
  eventId: string
  summary: string
}

export const approvalQueue = atom<ApprovalQueueItem[]>([])

export const headApproval = computed(approvalQueue, queue => queue[0] ?? null)

export function enqueueApproval(item: ApprovalQueueItem) {
  const current = approvalQueue.get()
  if (current.some(entry => entry.eventId === item.eventId)) return
  approvalQueue.set([...current, item])
}

export function dequeueApproval(eventId: string) {
  approvalQueue.set(approvalQueue.get().filter(item => item.eventId !== eventId))
}

export function clearApprovalQueueForTask(taskId: string) {
  approvalQueue.set(approvalQueue.get().filter(item => item.taskId !== taskId))
}

export function syncApprovalQueueFromEvents(taskId: string, events: TaskEvent[]) {
  const pending = events
    .filter(event => event.kind === 'approval_request')
    .map(event => ({
      taskId,
      eventId: event.id,
      summary: eventApprovalSummary(event)
    }))

  const responded = new Set(
    events
      .filter(event => event.kind === 'approval_response')
      .map(event => String((event.payload as { request_event_id?: string }).request_event_id ?? ''))
      .filter(Boolean)
  )

  const open = pending.filter(item => !responded.has(item.eventId))
  const others = approvalQueue.get().filter(item => item.taskId !== taskId)
  approvalQueue.set([...others, ...open])
}

export async function resolveHeadApproval(approved: boolean, reason?: string) {
  const head = headApproval.get()
  if (!head) return
  await approveTask(head.taskId, head.eventId, approved, reason)
  dequeueApproval(head.eventId)
}
