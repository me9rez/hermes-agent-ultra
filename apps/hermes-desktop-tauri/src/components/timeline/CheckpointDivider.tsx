interface CheckpointDividerProps {
  label?: string
}

export function CheckpointDivider({ label }: CheckpointDividerProps) {
  return <hr className="terra-checkpoint-divider" aria-label={label ?? 'Checkpoint'} />
}

export default CheckpointDivider
