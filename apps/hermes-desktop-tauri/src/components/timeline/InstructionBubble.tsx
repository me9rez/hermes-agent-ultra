interface InstructionBubbleProps {
  text: string
}

export function InstructionBubble({ text }: InstructionBubbleProps) {
  return <div className="terra-instruction-bubble">{text}</div>
}

export default InstructionBubble
