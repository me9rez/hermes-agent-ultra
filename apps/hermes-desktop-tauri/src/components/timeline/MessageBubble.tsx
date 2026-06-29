interface MessageBubbleProps {
  text: string
  streaming?: boolean
}

export function MessageBubble({ text, streaming }: MessageBubbleProps) {
  return (
    <div className="terra-message-bubble" data-streaming={streaming ? 'true' : undefined}>
      {text}
    </div>
  )
}

export default MessageBubble
