export type ThinkParseState = 'idle' | 'in_tag' | 'done'

export interface ThinkParseResult {
  visible: string
  thinking: string
  state: ThinkParseState
}

const OPEN = '<' + 'redacted_thinking' + '>'
const CLOSE = '</' + 'redacted_thinking' + '>'

function trailingPrefixMatch(text: string, needle: string): number {
  const max = Math.min(text.length, needle.length - 1)
  for (let len = max; len > 0; len -= 1) {
    if (needle.startsWith(text.slice(-len))) {
      return len
    }
  }
  return 0
}

export class ThinkTagParser {
  private carry = ''

  reset() {
    this.carry = ''
  }

  feed(chunk: string): ThinkParseResult {
    const input = this.carry + chunk
    this.carry = ''

    const openIdx = input.indexOf(OPEN)
    if (openIdx === -1) {
      const partial = trailingPrefixMatch(input, OPEN)
      if (partial > 0) {
        this.carry = input.slice(-partial)
        return { visible: input.slice(0, -partial), thinking: '', state: 'idle' }
      }
      return { visible: input, thinking: '', state: 'idle' }
    }

    const afterOpen = input.slice(openIdx + OPEN.length)
    const closeIdx = afterOpen.indexOf(CLOSE)
    if (closeIdx === -1) {
      const partial = trailingPrefixMatch(afterOpen, CLOSE)
      const thinking = partial > 0 ? afterOpen.slice(0, -partial) : afterOpen
      if (partial > 0) {
        this.carry = afterOpen.slice(-partial)
      }
      return {
        visible: input.slice(0, openIdx),
        thinking,
        state: 'in_tag'
      }
    }

    const thinking = afterOpen.slice(0, closeIdx)
    const tail = afterOpen.slice(closeIdx + CLOSE.length)
    const tailResult = this.feed(tail)
    return {
      visible: input.slice(0, openIdx) + tailResult.visible,
      thinking: thinking + (tailResult.thinking ? `\n${tailResult.thinking}` : ''),
      state: tailResult.state === 'idle' ? 'done' : tailResult.state
    }
  }

  flush(): ThinkParseResult {
    if (!this.carry) {
      return { visible: '', thinking: '', state: 'idle' }
    }
    const pending = this.carry
    this.carry = ''
    return { visible: pending, thinking: '', state: 'idle' }
  }
}

export function parseThinkContent(text: string): ThinkParseResult {
  const parser = new ThinkTagParser()
  const parsed = parser.feed(text)
  const flushed = parser.flush()
  return {
    visible: parsed.visible + flushed.visible,
    thinking: parsed.thinking + flushed.thinking,
    state: parsed.state
  }
}

export function parseThinkChunk(buffer: string, chunk: string): ThinkParseResult {
  const parser = new ThinkTagParser()
  const first = parser.feed(buffer)
  const second = parser.feed(chunk)
  const flushed = parser.flush()
  return {
    visible: first.visible + second.visible + flushed.visible,
    thinking: [first.thinking, second.thinking, flushed.thinking].filter(Boolean).join('\n'),
    state: second.state === 'idle' && flushed.state === 'idle' ? first.state : second.state
  }
}
