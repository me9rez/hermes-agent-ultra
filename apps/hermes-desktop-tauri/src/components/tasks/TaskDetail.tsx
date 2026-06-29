import type { ReactNode } from 'react'

import { Composer } from '@/components/chat/Composer'
import { TaskHeader } from '@/components/tasks/TaskHeader'
import { Timeline } from '@/components/tasks/Timeline'
import type { Task, TaskEvent } from '@/types/task'

interface TaskDetailProps {
  task: Task
  events: TaskEvent[]
  eventsLoading?: boolean
  composerValue: string
  onComposerChange: (value: string) => void
  onComposerSubmit: () => void
  onComposerStop?: () => void
  onTitleChange?: (title: string) => void
  rightRail?: ReactNode
}

export function TaskDetail({
  task,
  events,
  eventsLoading,
  composerValue,
  onComposerChange,
  onComposerSubmit,
  onComposerStop,
  onTitleChange,
  rightRail
}: TaskDetailProps) {
  const running = task.status === 'running' || task.status === 'pending'

  return (
    <section className="terra-task-detail" data-task-id={task.id}>
      <TaskHeader
        title={task.title}
        status={task.status}
        verticalId={task.vertical ?? undefined}
        onTitleChange={onTitleChange}
      />
      <div className="terra-task-detail__body">
        <Timeline taskId={task.id} events={events} loading={eventsLoading} />
        {rightRail}
      </div>
      <Composer
        value={composerValue}
        onChange={onComposerChange}
        onSubmit={onComposerSubmit}
        onStop={onComposerStop}
        running={running}
      />
    </section>
  )
}

export default TaskDetail
