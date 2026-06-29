import { useState } from 'react'

export type AkshareMode = 'cloud' | 'local'

const STORAGE_KEY = 'terra.settings.akshareMode.v1'

export function readAkshareMode(): AkshareMode {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw === 'local') return 'local'
  } catch {
    /* ignore */
  }
  return 'cloud'
}

export function writeAkshareMode(mode: AkshareMode): void {
  localStorage.setItem(STORAGE_KEY, mode)
}

interface AkshareDataSourceSettingsProps {
  localAvailable?: boolean
  onModeChange?: (mode: AkshareMode) => void
}

export function AkshareDataSourceSettings({
  localAvailable = false,
  onModeChange
}: AkshareDataSourceSettingsProps) {
  const [mode, setMode] = useState<AkshareMode>(() => readAkshareMode())

  const select = (next: AkshareMode) => {
    if (next === 'local' && !localAvailable) return
    setMode(next)
    writeAkshareMode(next)
    onModeChange?.(next)
  }

  return (
    <section className="settings-datasource-akshare">
      <h3>Akshare</h3>
      <label>
        <input
          type="radio"
          name="akshare-mode"
          checked={mode === 'cloud'}
          onChange={() => select('cloud')}
        />
        Cloud (default)
      </label>
      <label>
        <input
          type="radio"
          name="akshare-mode"
          checked={mode === 'local'}
          disabled={!localAvailable}
          onChange={() => select('local')}
        />
        Local Python bridge
      </label>
      {!localAvailable && (
        <p className="settings-hint">
          Local mode requires Python + akshare. Install dependencies to enable.
        </p>
      )}
    </section>
  )
}

export default AkshareDataSourceSettings
