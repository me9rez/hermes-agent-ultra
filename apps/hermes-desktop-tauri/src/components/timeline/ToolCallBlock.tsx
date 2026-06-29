interface ToolCallBlockProps {
  toolName: string
  args?: string
  children?: React.ReactNode
}

export function ToolCallBlock({ toolName, args, children }: ToolCallBlockProps) {
  return (
    <details className="terra-tool-call-block">
      <summary>{toolName}</summary>
      {args && <pre>{args}</pre>}
      {children}
    </details>
  )
}

export default ToolCallBlock
