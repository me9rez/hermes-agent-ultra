import { useEffect, useState } from 'react'
import { hasColdCache } from '@/lib/cache-first-bootstrap'

interface SplashScreenProps {
  onReady: () => void
  progress?: number
  message?: string
}

export function SplashScreen({ onReady, progress = 0, message }: SplashScreenProps) {
  const [visible, setVisible] = useState(!hasColdCache())

  useEffect(() => {
    if (!visible) {
      onReady()
    }
  }, [visible, onReady])

  if (!visible) return null

  return (
    <div className="terra-splash" role="status" aria-live="polite">
      <div className="terra-splash__logo" />
      <p className="terra-splash__message">{message ?? 'Starting Terra...'}</p>
      <progress max={100} value={progress} />
      <button type="button" onClick={() => setVisible(false)}>
        Skip
      </button>
    </div>
  )
}

export default SplashScreen
