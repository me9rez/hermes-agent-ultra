interface TaskOutlineProps {
  items?: { id: string; label: string; depth?: number }[]
  onSelect?: (id: string) => void
}

export function TaskOutline({ items = [], onSelect }: TaskOutlineProps) {
  return (
    <nav className="terra-task-outline">
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          style={{ paddingLeft: `${(item.depth ?? 0) * 12}px` }}
          onClick={() => onSelect?.(item.id)}
        >
          {item.label}
        </button>
      ))}
    </nav>
  )
}

export default TaskOutline
