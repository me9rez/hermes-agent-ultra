export interface MarkdownProgressiveResult {
  source: string
  incomplete: boolean
  unclosedFence: boolean
}

function countUnescapedBackticks(line: string): number {
  let count = 0
  let escaped = false
  for (const char of line) {
    if (escaped) {
      escaped = false
      continue
    }
    if (char === '\\') {
      escaped = true
      continue
    }
    if (char === '`') {
      count += 1
    }
  }
  return count
}

export function renderProgressiveMarkdown(source: string): MarkdownProgressiveResult {
  const lines = source.split('\n')
  let fenceOpen = false

  for (const line of lines) {
    const ticks = countUnescapedBackticks(line)
    if (ticks >= 3 && ticks % 2 === 1) {
      fenceOpen = !fenceOpen
    }
  }

  const trailingInlineTick = /(?:^|[^\\])`[^`\n]*$/.test(source)
  const unclosedFence = fenceOpen
  let display = source

  if (unclosedFence) {
    display = `${source}\n\`\`\``
  }

  return {
    source: display,
    incomplete: unclosedFence || trailingInlineTick,
    unclosedFence
  }
}
