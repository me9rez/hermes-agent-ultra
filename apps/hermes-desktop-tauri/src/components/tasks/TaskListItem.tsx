interface TaskListItemProps {
  taskId: string
  title: string
  status?: string
  unread?: boolean
  onClick?: () => void
}

export function TaskListItem({ taskId, title, status, unread, onClick }: TaskListItemProps) {
  return (
    <button type="button" className="terra-task-list-item" data-task-id={taskId} onClick={onClick}>
      <span className="terra-task-list-item__title">{title}</span>
      {status && <span className="terra-task-list-item__status">{status}</span>}
      {unread && <span className="terra-task-list-item__unread" aria-label="Unread" />}
    </button>
  )
}

export default TaskListItem
