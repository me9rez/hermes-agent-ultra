interface TaskDetailProps {
  taskId: string
  children?: React.ReactNode
}

export function TaskDetail({ taskId, children }: TaskDetailProps) {
  return (
    <section className="terra-task-detail" data-task-id={taskId}>
      {children}
    </section>
  )
}

export default TaskDetail
