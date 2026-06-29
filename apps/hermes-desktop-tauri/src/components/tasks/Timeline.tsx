interface TimelineProps {
  taskId: string
  children?: React.ReactNode
}

export function Timeline({ taskId, children }: TimelineProps) {
  return (
    <div className="terra-timeline" data-task-id={taskId}>
      {children}
    </div>
  )
}

export default Timeline
