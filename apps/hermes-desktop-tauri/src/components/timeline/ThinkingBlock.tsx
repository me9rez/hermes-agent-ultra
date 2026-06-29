interface ThinkingBlockProps {
  text: string
  durationMs?: number
  defaultCollapsed?: boolean
}

export function ThinkingBlock({ text, durationMs, defaultCollapsed = true }: ThinkingBlockProps) {
  return (
    <details className="terra-thinking-block" open={!defaultCollapsed}>
      <summary>Thinking{durationMs != null ? ` (${durationMs}ms)` : ''}</summary>
      <p>{text}</p>
    </details>
  )
}

export default ThinkingBlock
