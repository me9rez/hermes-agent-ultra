interface TaskListRailProps {
  taskIds?: string[]
  selectedId?: string
  onSelect?: (taskId: string) => void
}

export function TaskListRail({ taskIds = [], selectedId, onSelect }: TaskListRailProps) {
  return (
    <aside className="terra-task-list-rail">
      {taskIds.map((id) => (
        <button
          key={id}
          type="button"
          className={selectedId === id ? 'is-selected' : undefined}
          onClick={() => onSelect?.(id)}
        >
          {id}
        </button>
      ))}
    </aside>
  )
}

export default TaskListRail
