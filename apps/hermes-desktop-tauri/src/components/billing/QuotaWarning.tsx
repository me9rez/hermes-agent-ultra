import { useT } from '@/i18n/useT'

interface QuotaWarningProps {
  percentRemaining: number
  onUpgrade?: () => void
}

export function QuotaWarning({ percentRemaining, onUpgrade }: QuotaWarningProps) {
  const t = useT('billing')

  if (percentRemaining >= 10) return null

  return (
    <div className="terra-quota-warning" role="alert">
      <p>{t('quota.low', 'Monthly quota is almost used up.')}</p>
      <button type="button" onClick={onUpgrade}>
        {t('quota.upgrade', 'Upgrade to Pro')}
      </button>
    </div>
  )
}

export default QuotaWarning
