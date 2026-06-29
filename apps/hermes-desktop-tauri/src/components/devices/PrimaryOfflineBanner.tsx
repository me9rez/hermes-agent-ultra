import { useEffect, useState } from 'react'

import { useT } from '@/i18n/useT'

export function PrimaryOfflineBanner() {
  const t = useT('app')
  const [primaryOffline, setPrimaryOffline] = useState(false)

  useEffect(() => {
    let cancelled = false
    void fetch('/api/devices/relay/status')
      .then(res => (res.ok ? res.json() : null))
      .then(body => {
        if (cancelled || !body) return
        setPrimaryOffline(body.connected === false)
      })
      .catch(() => {
        if (!cancelled) setPrimaryOffline(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  if (!primaryOffline) return null

  return (
    <div className="terra-primary-offline-banner" role="status">
      {t(
        'primaryOffline',
        'Primary desktop is offline. Some actions are read-only until it reconnects.'
      )}
    </div>
  )
}

export default PrimaryOfflineBanner
