interface VerticalPickerProps {
  onSelect?: (verticalId: string) => void
}

export function VerticalPicker({ onSelect }: VerticalPickerProps) {
  return (
    <div className="terra-vertical-picker" role="listbox">
      <button type="button" onClick={() => onSelect?.('trader')}>
        Trader
      </button>
      <button type="button" onClick={() => onSelect?.('knowledge')}>
        Knowledge
      </button>
    </div>
  )
}

export default VerticalPicker
