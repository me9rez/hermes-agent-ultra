import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import {
  approveTask,
  cancelTask,
  continueTask,
  createTask,
  getTask,
  listTaskEvents,
  listTasks,
  listVerticals
} from '@/lib/task-api'
import type { CreateTaskRequest, TaskListParams } from '@/types/task'

export const taskQueryKeys = {
  all: ['terra-tasks'] as const,
  list: (params?: TaskListParams) => [...taskQueryKeys.all, 'list', params ?? {}] as const,
  detail: (taskId: string) => [...taskQueryKeys.all, 'detail', taskId] as const,
  events: (taskId: string) => [...taskQueryKeys.all, 'events', taskId] as const,
  verticals: (search?: string) => ['terra-verticals', search ?? ''] as const
}

export function useTasksQuery(params?: TaskListParams) {
  return useQuery({
    queryKey: taskQueryKeys.list(params),
    queryFn: () => listTasks(params),
    refetchInterval: 5_000
  })
}

export function useTaskQuery(taskId: string | null) {
  return useQuery({
    queryKey: taskQueryKeys.detail(taskId ?? ''),
    queryFn: () => getTask(taskId!),
    enabled: Boolean(taskId)
  })
}

export function useTaskEventsQuery(taskId: string | null) {
  return useQuery({
    queryKey: taskQueryKeys.events(taskId ?? ''),
    queryFn: () => listTaskEvents(taskId!),
    enabled: Boolean(taskId),
    refetchInterval: (query) => {
      const events = query.state.data?.events
      const streaming = events?.some(event => event.streaming)
      return streaming ? 1_000 : 3_000
    }
  })
}

export function useVerticalsQuery(search?: string) {
  return useQuery({
    queryKey: taskQueryKeys.verticals(search),
    queryFn: () => listVerticals(search ? { search } : undefined)
  })
}

export function useCreateTaskMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (body: CreateTaskRequest) => createTask(body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.all })
    }
  })
}

export function useContinueTaskMutation(taskId: string | null) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (instruction: string) => continueTask(taskId!, instruction),
    onSuccess: () => {
      if (!taskId) return
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.detail(taskId) })
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.events(taskId) })
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.all })
    }
  })
}

export function useCancelTaskMutation(taskId: string | null) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => cancelTask(taskId!),
    onSuccess: () => {
      if (!taskId) return
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.detail(taskId) })
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.all })
    }
  })
}

export function useApproveTaskMutation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({
      taskId,
      eventId,
      approved,
      reason
    }: {
      taskId: string
      eventId: string
      approved: boolean
      reason?: string
    }) => approveTask(taskId, eventId, approved, reason),
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.detail(variables.taskId) })
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.events(variables.taskId) })
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.all })
    }
  })
}
