interface TaskListItemProps {
  taskId: string
  title: string
  status?: string
  unread?: boolean
  selected?: boolean
  onClick?: () => void
}

export function TaskListItem({ taskId, title, status, unread, selected, onClick }: TaskListItemProps) {
  return (
    <button
      type="button"
      className="terra-task-list-item"
      data-task-id={taskId}
      data-selected={selected ? 'true' : undefined}
      onClick={onClick}
    >
      <span className="terra-task-list-item__title">{title}</span>
      {status ? <span className="terra-task-list-item__status">{status}</span> : null}
      {unread ? <span className="terra-task-list-item__unread" aria-label="Unread" /> : null}
    </button>
  )
}

export default TaskListItem
