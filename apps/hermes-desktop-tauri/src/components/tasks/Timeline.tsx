import { useVirtualizer } from '@tanstack/react-virtual'
import { useEffect, useRef } from 'react'

import { TaskEventRow, useRenderableEvents } from '@/components/timeline/TaskEventRow'
import { syncApprovalQueueFromEvents } from '@/stores/approval-queue'
import type { TaskEvent } from '@/types/task'

interface TimelineProps {
  taskId: string
  events: TaskEvent[]
  loading?: boolean
}

export function Timeline({ taskId, events, loading }: TimelineProps) {
  const parentRef = useRef<HTMLDivElement>(null)
  const renderable = useRenderableEvents(events)

  useEffect(() => {
    syncApprovalQueueFromEvents(taskId, events)
  }, [taskId, events])

  const virtualizer = useVirtualizer({
    count: renderable.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 96,
    overscan: 6
  })

  return (
    <div className="terra-timeline" data-task-id={taskId} ref={parentRef}>
      {loading ? <p className="terra-timeline__loading">...</p> : null}
      <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
        {virtualizer.getVirtualItems().map(item => {
          const row = renderable[item.index]
          if (!row) return null

          return (
            <article
              key={row.event.id}
              id={row.event.anchor_slug}
              className="terra-timeline__event"
              data-kind={row.event.kind}
              style={{
                position: 'absolute',
                top: 0,
                left: 0,
                width: '100%',
                transform: `translateY(${item.start}px)`
              }}
            >
              <TaskEventRow event={row.event} childResults={row.childResults} />
            </article>
          )
        })}
      </div>
    </div>
  )
}

export default Timeline
