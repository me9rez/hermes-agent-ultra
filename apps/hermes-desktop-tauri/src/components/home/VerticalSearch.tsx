interface VerticalSearchProps {
  query?: string
  onQueryChange?: (query: string) => void
}

export function VerticalSearch({ query = '', onQueryChange }: VerticalSearchProps) {
  return (
    <div className="terra-vertical-search">
      <input
        type="search"
        value={query}
        placeholder="Search verticals"
        onChange={(e) => onQueryChange?.(e.target.value)}
      />
    </div>
  )
}

export default VerticalSearch
