import { useMemo } from 'react'

import { ApprovalRequestCard } from '@/components/timeline/ApprovalRequestCard'
import { ArtifactCard } from '@/components/timeline/ArtifactCard'
import { CheckpointDivider } from '@/components/timeline/CheckpointDivider'
import { ErrorCard } from '@/components/timeline/ErrorCard'
import { InstructionBubble } from '@/components/timeline/InstructionBubble'
import { MessageBubble } from '@/components/timeline/MessageBubble'
import { PlanCard } from '@/components/timeline/PlanCard'
import { SubagentSpawnCard } from '@/components/timeline/SubagentSpawnCard'
import { SystemEvent } from '@/components/timeline/SystemEvent'
import { ThinkingBlock } from '@/components/timeline/ThinkingBlock'
import { ToolCallBlock } from '@/components/timeline/ToolCallBlock'
import { ToolResultBlock } from '@/components/timeline/ToolResultBlock'
import { useApproveTaskMutation } from '@/hooks/use-task-queries'
import {
  eventApprovalSummary,
  eventArtifactName,
  eventPayloadSteps,
  eventPayloadText,
  eventToolArgs,
  eventToolName,
  eventToolResult
} from '@/lib/task-event-utils'
import { parseThinkContent } from '@/lib/stream-parser/think-tag'
import { dequeueApproval } from '@/stores/approval-queue'
import type { TaskEvent } from '@/types/task'

interface TaskEventRowProps {
  event: TaskEvent
  childResults?: TaskEvent[]
}

export function TaskEventRow({ event, childResults = [] }: TaskEventRowProps) {
  const approveMutation = useApproveTaskMutation()

  const handleApprove = (approved: boolean) => {
    approveMutation.mutate(
      { taskId: event.task_id, eventId: event.id, approved },
      { onSuccess: () => dequeueApproval(event.id) }
    )
  }

  switch (event.kind) {
    case 'instruction':
      return <InstructionBubble text={eventPayloadText(event)} />
    case 'message': {
      const parsed = parseThinkContent(eventPayloadText(event))
      return (
        <>
          {parsed.thinking ? (
            <ThinkingBlock text={parsed.thinking} durationMs={event.duration_ms ?? undefined} />
          ) : null}
          <MessageBubble text={parsed.visible} streaming={event.streaming} />
        </>
      )
    }
    case 'plan':
      return <PlanCard steps={eventPayloadSteps(event)} />
    case 'thinking':
      return (
        <ThinkingBlock
          text={eventPayloadText(event)}
          durationMs={event.duration_ms ?? undefined}
          defaultCollapsed={event.collapsed_by_default}
        />
      )
    case 'tool_call':
      return (
        <ToolCallBlock toolName={eventToolName(event)} args={eventToolArgs(event)}>
          {childResults.map(child => (
            <ToolResultBlock key={child.id} result={eventToolResult(child)} />
          ))}
        </ToolCallBlock>
      )
    case 'tool_result':
      return <ToolResultBlock result={eventToolResult(event)} />
    case 'subagent_spawn':
      return <SubagentSpawnCard role={eventPayloadText(event) || event.toc_label || 'subagent'} />
    case 'artifact':
      return <ArtifactCard name={eventArtifactName(event)} />
    case 'approval_request':
      return (
        <ApprovalRequestCard
          summary={eventApprovalSummary(event)}
          onApprove={() => handleApprove(true)}
          onReject={() => handleApprove(false)}
        />
      )
    case 'checkpoint':
      return <CheckpointDivider label={event.toc_label ?? undefined} />
    case 'error':
      return <ErrorCard message={eventPayloadText(event) || 'Error'} details={JSON.stringify(event.payload, null, 2)} />
    case 'approval_response':
    case 'system':
      return <SystemEvent text={eventPayloadText(event) || event.toc_label || event.kind} />
    default: {
      const _exhaustive: never = event.kind
      return <SystemEvent text={String(_exhaustive)} />
    }
  }
}

export function groupToolResults(events: TaskEvent[]): Map<string, TaskEvent[]> {
  const resultsByParent = new Map<string, TaskEvent[]>()
  for (const event of events) {
    if (event.kind !== 'tool_result' || !event.parent_event_id) continue
    const key = event.parent_event_id
    const bucket = resultsByParent.get(key) ?? []
    bucket.push(event)
    resultsByParent.set(key, bucket)
  }
  return resultsByParent
}

export function useRenderableEvents(events: TaskEvent[]) {
  return useMemo(() => {
    const resultsByParent = groupToolResults(events)
    const nestedResultIds = new Set(
      [...resultsByParent.values()].flat().map(event => event.id)
    )
    return events
      .filter(event => !nestedResultIds.has(event.id))
      .map(event => ({
        event,
        childResults: event.kind === 'tool_call' ? resultsByParent.get(event.id) ?? [] : []
      }))
  }, [events])
}
