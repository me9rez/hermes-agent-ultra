import { useT } from '@/i18n/useT'
import type { VerticalMeta } from '@/types/task'

interface VerticalPickerProps {
  verticals: VerticalMeta[]
  loading?: boolean
  onSelect?: (verticalId: string) => void
}

export function VerticalPicker({ verticals, loading, onSelect }: VerticalPickerProps) {
  const t = useT('vertical')

  if (loading) {
    return <div className="terra-vertical-picker terra-vertical-picker--loading">{t('picker')}</div>
  }

  return (
    <div className="terra-vertical-picker" role="listbox" aria-label={t('picker')}>
      {verticals.map(vertical => (
        <button
          key={vertical.id}
          type="button"
          className="terra-vertical-picker__card"
          onClick={() => onSelect?.(vertical.id)}
        >
          <span className="terra-vertical-picker__icon" aria-hidden>
            {vertical.icon}
          </span>
          <span className="terra-vertical-picker__title">{vertical.id}</span>
          <span className="terra-vertical-picker__category">{vertical.category}</span>
        </button>
      ))}
    </div>
  )
}

export default VerticalPicker
