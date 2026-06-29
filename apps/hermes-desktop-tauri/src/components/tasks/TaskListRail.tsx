import { useVirtualizer } from '@tanstack/react-virtual'
import { useMemo, useRef } from 'react'

import { TaskListItem } from '@/components/tasks/TaskListItem'
import { useT } from '@/i18n/useT'
import { TASK_STATUS_GROUPS, type Task, type TaskStatus } from '@/types/task'

interface TaskListRailProps {
  tasks: Task[]
  selectedId?: string | null
  unreadIds?: Set<string>
  loading?: boolean
  onSelect?: (taskId: string) => void
}

function groupTasks(tasks: Task[]) {
  const buckets = new Map<string, Task[]>()
  for (const group of TASK_STATUS_GROUPS) {
    buckets.set(group.key, [])
  }

  for (const task of tasks) {
    const group = TASK_STATUS_GROUPS.find(entry => entry.statuses.includes(task.status as TaskStatus))
    const key = group?.key ?? 'done'
    buckets.get(key)?.push(task)
  }

  return TASK_STATUS_GROUPS.map(group => ({
    key: group.key,
    tasks: buckets.get(group.key) ?? []
  })).filter(group => group.tasks.length > 0)
}

export function TaskListRail({ tasks, selectedId, unreadIds, loading, onSelect }: TaskListRailProps) {
  const t = useT('task')
  const parentRef = useRef<HTMLDivElement>(null)
  const groups = useMemo(() => groupTasks(tasks), [tasks])
  const flatRows = useMemo(
    () =>
      groups.flatMap(group => [
        { type: 'header' as const, key: `header-${group.key}`, label: group.key },
        ...group.tasks.map(task => ({ type: 'task' as const, key: task.id, task }))
      ]),
    [groups]
  )

  const virtualizer = useVirtualizer({
    count: flatRows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: index => (flatRows[index]?.type === 'header' ? 28 : 56),
    overscan: 8
  })

  return (
    <aside className="terra-task-list-rail" aria-label={t('list')}>
      <header className="terra-task-list-rail__header">{t('list')}</header>
      {loading ? <p className="terra-task-list-rail__loading">...</p> : null}
      <div ref={parentRef} className="terra-task-list-rail__scroll">
        <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
          {virtualizer.getVirtualItems().map(item => {
            const row = flatRows[item.index]
            if (!row) return null

            return (
              <div
                key={row.key}
                className="terra-task-list-rail__row"
                style={{
                  position: 'absolute',
                  top: 0,
                  left: 0,
                  width: '100%',
                  transform: `translateY(${item.start}px)`
                }}
              >
                {row.type === 'header' ? (
                  <h3 className="terra-task-list-rail__group">{row.label}</h3>
                ) : (
                  <TaskListItem
                    taskId={row.task.id}
                    title={row.task.title}
                    status={row.task.status}
                    unread={unreadIds?.has(row.task.id)}
                    selected={selectedId === row.task.id}
                    onClick={() => onSelect?.(row.task.id)}
                  />
                )}
              </div>
            )
          })}
        </div>
      </div>
    </aside>
  )
}

export default TaskListRail
