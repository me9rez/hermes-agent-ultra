import { useT } from '@/i18n/useT'

interface ToolBudgetPanelProps {
  tier?: string
}

export function ToolBudgetPanel({ tier = 'free' }: ToolBudgetPanelProps) {
  const t = useT('billing')

  const rows = [
    { id: 'web_search', used: 0, limit: tier === 'free' ? 50 : 500 },
    { id: 'vision', used: 0, limit: tier === 'free' ? 20 : 300 },
    { id: 'computer_use', used: 0, limit: tier === 'free' ? 0 : 50 },
    { id: 'execute_code', used: 0, limit: tier === 'free' ? 200 : 9999 }
  ]

  return (
    <section className="terra-tool-budget">
      <h3>{t('toolBudget.title', 'Tool usage')}</h3>
      <ul>
        {rows.map(row => (
          <li key={row.id}>
            <span>{row.id}</span>
            <span>
              {row.used} / {row.limit === 9999 ? '∞' : row.limit}
            </span>
          </li>
        ))}
      </ul>
    </section>
  )
}

export default ToolBudgetPanel
