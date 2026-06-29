interface TaskHeaderProps {
  title: string
  status?: string
  verticalId?: string
  onTitleChange?: (title: string) => void
}

export function TaskHeader({ title, status, verticalId, onTitleChange }: TaskHeaderProps) {
  return (
    <header className="terra-task-header">
      <input
        className="terra-task-header__title"
        value={title}
        onChange={(e) => onTitleChange?.(e.target.value)}
      />
      {status && <span className="terra-task-header__status">{status}</span>}
      {verticalId && <span className="terra-task-header__vertical">{verticalId}</span>}
    </header>
  )
}

export default TaskHeader
