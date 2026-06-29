import { useStore } from '@nanostores/react'
import { useCallback, useState } from 'react'

import { ApprovalModal } from '@/components/approval/ApprovalModal'
import { VerticalPicker } from '@/components/home/VerticalPicker'
import { VerticalSearch } from '@/components/home/VerticalSearch'
import { TaskDetail } from '@/components/tasks/TaskDetail'
import { TaskListRail } from '@/components/tasks/TaskListRail'
import { TaskMinimap } from '@/components/tasks/TaskMinimap'
import {
  useCancelTaskMutation,
  useContinueTaskMutation,
  useCreateTaskMutation,
  useTaskEventsQuery,
  useTaskQuery,
  useTasksQuery,
  useVerticalsQuery
} from '@/hooks/use-task-queries'
import { minimapColorForKind } from '@/lib/task-event-utils'
import { headApproval, resolveHeadApproval } from '@/stores/approval-queue'
import { $activeTaskId, $unreadTaskIds, setActiveTaskId, setSelectedVerticalId } from '@/stores/active-task'

export default function TerraApp() {
  const activeTaskId = useStore($activeTaskId)
  const unreadIds = useStore($unreadTaskIds)
  const pendingApproval = useStore(headApproval)

  const [verticalQuery, setVerticalQuery] = useState('')
  const [composerDraft, setComposerDraft] = useState('')

  const tasksQuery = useTasksQuery()
  const taskQuery = useTaskQuery(activeTaskId)
  const eventsQuery = useTaskEventsQuery(activeTaskId)
  const verticalsQuery = useVerticalsQuery(verticalQuery)

  const createTaskMutation = useCreateTaskMutation()
  const continueTaskMutation = useContinueTaskMutation(activeTaskId)
  const cancelTaskMutation = useCancelTaskMutation(activeTaskId)

  const handleVerticalSelect = useCallback(
    async (verticalId: string) => {
      setSelectedVerticalId(verticalId)
      const result = await createTaskMutation.mutateAsync({
        title: `New ${verticalId} task`,
        vertical: verticalId,
        instruction: ''
      })
      setActiveTaskId(result.task.id)
      setComposerDraft('')
    },
    [createTaskMutation]
  )

  const handleComposerSubmit = useCallback(async () => {
    const instruction = composerDraft.trim()
    if (!instruction) return

    if (!activeTaskId) {
      const vertical = verticalsQuery.data?.verticals[0]?.id
      const result = await createTaskMutation.mutateAsync({
        title: instruction.slice(0, 80),
        vertical,
        instruction
      })
      setActiveTaskId(result.task.id)
      setComposerDraft('')
      return
    }

    await continueTaskMutation.mutateAsync(instruction)
    setComposerDraft('')
  }, [activeTaskId, composerDraft, continueTaskMutation, createTaskMutation, verticalsQuery.data?.verticals])

  const minimapAnchors =
    eventsQuery.data?.events.map(event => ({
      id: event.anchor_slug,
      color: minimapColorForKind(event.kind)
    })) ?? []

  return (
    <div className="terra-shell">
      <TaskListRail
        tasks={tasksQuery.data?.tasks ?? []}
        selectedId={activeTaskId}
        unreadIds={unreadIds}
        loading={tasksQuery.isLoading}
        onSelect={setActiveTaskId}
      />

      <main className="terra-shell__main">
        {!activeTaskId ? (
          <section className="terra-home">
            <VerticalSearch query={verticalQuery} onQueryChange={setVerticalQuery} />
            <VerticalPicker
              verticals={verticalsQuery.data?.verticals ?? []}
              loading={verticalsQuery.isLoading}
              onSelect={verticalId => void handleVerticalSelect(verticalId)}
            />
          </section>
        ) : taskQuery.data ? (
          <TaskDetail
            task={taskQuery.data}
            events={eventsQuery.data?.events ?? []}
            eventsLoading={eventsQuery.isLoading}
            composerValue={composerDraft}
            onComposerChange={setComposerDraft}
            onComposerSubmit={() => void handleComposerSubmit()}
            onComposerStop={() => void cancelTaskMutation.mutate()}
            rightRail={
              <TaskMinimap
                anchors={minimapAnchors}
                onJump={anchorId => document.getElementById(anchorId)?.scrollIntoView({ behavior: 'smooth' })}
              />
            }
          />
        ) : (
          <p className="terra-shell__loading">...</p>
        )}
      </main>

      <ApprovalModal
        open={Boolean(pendingApproval)}
        summary={pendingApproval?.summary ?? ''}
        onApprove={() => void resolveHeadApproval(true)}
        onReject={() => void resolveHeadApproval(false)}
        onClose={() => undefined}
      />
    </div>
  )
}
