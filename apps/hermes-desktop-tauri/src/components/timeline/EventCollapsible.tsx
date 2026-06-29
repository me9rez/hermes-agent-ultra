interface EventCollapsibleProps {
  title: string
  defaultOpen?: boolean
  children: React.ReactNode
}

export function EventCollapsible({ title, defaultOpen, children }: EventCollapsibleProps) {
  return (
    <details className="terra-event-collapsible" open={defaultOpen}>
      <summary>{title}</summary>
      {children}
    </details>
  )
}

export default EventCollapsible
