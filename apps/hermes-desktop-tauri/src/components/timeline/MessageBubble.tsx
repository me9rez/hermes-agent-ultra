import { Streamdown } from 'streamdown'

import { renderProgressiveMarkdown } from '@/lib/stream-parser/markdown-progressive'

interface MessageBubbleProps {
  text: string
  streaming?: boolean
}

export function MessageBubble({ text, streaming }: MessageBubbleProps) {
  const { source, incomplete } = renderProgressiveMarkdown(text)

  return (
    <div className="terra-message-bubble" data-streaming={streaming ? 'true' : undefined}>
      <Streamdown controls={false} mode="static" parseIncompleteMarkdown={incomplete || Boolean(streaming)}>
        {source}
      </Streamdown>
    </div>
  )
}

export default MessageBubble
