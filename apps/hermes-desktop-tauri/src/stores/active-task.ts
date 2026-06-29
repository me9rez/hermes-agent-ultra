import { atom } from 'nanostores'

export const $activeTaskId = atom<string | null>(null)
export const $selectedVerticalId = atom<string | null>(null)
export const $unreadTaskIds = atom<Set<string>>(new Set())

export function setActiveTaskId(taskId: string | null) {
  $activeTaskId.set(taskId)
  if (taskId) {
    const unread = new Set($unreadTaskIds.get())
    unread.delete(taskId)
    $unreadTaskIds.set(unread)
  }
}

export function markTaskUnread(taskId: string) {
  if ($activeTaskId.get() === taskId) return
  const unread = new Set($unreadTaskIds.get())
  unread.add(taskId)
  $unreadTaskIds.set(unread)
}

export function setSelectedVerticalId(verticalId: string | null) {
  $selectedVerticalId.set(verticalId)
}
