import { useT } from '@/i18n/useT'

interface VerticalSearchProps {
  query?: string
  onQueryChange?: (query: string) => void
}

export function VerticalSearch({ query = '', onQueryChange }: VerticalSearchProps) {
  const t = useT('vertical')

  return (
    <div className="terra-vertical-search">
      <input
        type="search"
        value={query}
        placeholder={t('picker')}
        onChange={event => onQueryChange?.(event.target.value)}
      />
    </div>
  )
}

export default VerticalSearch
