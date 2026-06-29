interface SystemEventProps {
  text: string
}

export function SystemEvent({ text }: SystemEventProps) {
  return <p className="terra-system-event">{text}</p>
}

export default SystemEvent
